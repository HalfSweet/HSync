import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import { i18n } from '@/i18n'
import type { ImportRequest } from '@/ipc/bindings/ImportRequest'
import type { WaveformSummary } from '@/ipc/bindings/WaveformSummary'
import { useCaptureStore } from '@/stores/capture'

import ImportDialog from './ImportDialog.vue'

vi.mock('@/ipc/backend', () => ({
  getBackend: () => Promise.resolve({ systemMemory: () => Promise.resolve(16 * 2 ** 30) }),
}))

/** A 100 MHz export: top { clk, data[3:0], uart { tx, temperature } }. */
const summary: WaveformSummary = {
  format: 'vcd',
  samplerate: 100_000_000,
  sampleCount: 6,
  coarsened: false,
  unit: 1e-12,
  span: 50_000,
  finest: 10_000,
  items: [
    { depth: 0, name: 'top', kind: 'scope', signal: null, width: 0, range: null },
    { depth: 1, name: 'clk', kind: 'logic', signal: 0, width: 1, range: null },
    { depth: 1, name: 'data', kind: 'logic', signal: 1, width: 4, range: [3, 0] },
    { depth: 1, name: 'uart', kind: 'scope', signal: null, width: 0, range: null },
    { depth: 2, name: 'tx', kind: 'logic', signal: 2, width: 1, range: null },
    { depth: 2, name: 'temperature', kind: 'other', signal: null, width: 0, range: null },
  ],
}

function dialog() {
  const capture = useCaptureStore()
  capture.pendingImport = { path: '/tmp/ila.vcd', summary }
  const requests: ImportRequest[] = []
  capture.importWaveform = (request: ImportRequest) => {
    requests.push(request)
    return Promise.resolve(true)
  }
  let discarded = 0
  capture.discardImport = () => {
    discarded += 1
    return Promise.resolve()
  }
  const wrapper = mount(ImportDialog, { global: { plugins: [i18n] }, attachTo: document.body })
  const importButton = () =>
    wrapper
      .findAll('footer button')
      .find((button) => button.text() === i18n.global.t('importDialog.import'))!
  return { wrapper, requests, discarded: () => discarded, importButton }
}

describe('import dialog', () => {
  beforeEach(() => setActivePinia(createPinia()))

  it('lists the file as a tree and names the file', () => {
    const { wrapper } = dialog()
    expect(wrapper.text()).toContain('ila.vcd')
    const rows = wrapper.findAll('[role="treeitem"]')
    expect(rows.map((row) => row.text())).toEqual([
      'top',
      'clk',
      expect.stringContaining('data'),
      'uart',
      'tx',
      expect.stringContaining('temperature'),
    ])
    expect(rows[2]!.text()).toContain('[3:0]')
    // Reals cannot be imported.
    expect(rows[5]!.find('input').exists()).toBe(false)
    wrapper.unmount()
  })

  it('starts with every signal of a small file chosen and imports them in order', async () => {
    const { wrapper, requests, importButton } = dialog()
    expect(wrapper.text()).toContain('100 MHz')
    expect(wrapper.text()).toContain('6 channels')
    await importButton().trigger('click')
    expect(requests).toEqual([{ signals: [0, 1, 2], samplerate: null }])
    wrapper.unmount()
  })

  it('folds scopes and chooses by scope', async () => {
    const { wrapper, requests, importButton } = dialog()
    const uart = wrapper.findAll('[role="treeitem"]')[3]!
    await uart.find('input[type="checkbox"]').setValue(false)
    expect(wrapper.text()).toContain('5 channels')
    await uart.find('button').trigger('click')
    expect(wrapper.findAll('[role="treeitem"]')).toHaveLength(4)
    await importButton().trigger('click')
    expect(requests).toEqual([{ signals: [0, 1], samplerate: null }])
    wrapper.unmount()
  })

  it('searches signals and refuses an import without any', async () => {
    const { wrapper, importButton } = dialog()
    await wrapper.find('input[type="text"], input:not([type])').setValue('tx')
    expect(wrapper.findAll('[role="treeitem"]').map((row) => row.text())).toEqual([
      'top',
      'uart',
      'tx',
    ])
    const all = wrapper
      .findAll('button')
      .find((button) => button.text() === i18n.global.t('importDialog.none'))!
    await all.trigger('click')
    expect(wrapper.text()).toContain(i18n.global.t('importDialog.noSignal'))
    expect(importButton().attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })

  it('takes a sample rate of the user’s own', async () => {
    const { wrapper, requests, importButton } = dialog()
    const rate = wrapper.findAll('input').find((input) => input.element.value === '100 MHz')!
    await rate.setValue('25 MHz')
    expect(wrapper.text()).toContain('25 MHz')
    expect(wrapper.text()).toContain('2 samples')
    expect(wrapper.text()).toContain(i18n.global.t('importDialog.coarsened'))
    await importButton().trigger('click')
    expect(requests).toEqual([{ signals: [0, 1, 2], samplerate: 25_000_000 }])
    await rate.setValue('fast')
    expect(wrapper.text()).toContain(i18n.global.t('importDialog.badSamplerate'))
    expect(importButton().attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })

  it('closes on Escape by forgetting the file', async () => {
    const { wrapper, discarded } = dialog()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    await nextTick()
    expect(discarded()).toBe(1)
    wrapper.unmount()
  })
})
