import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import { i18n } from '@/i18n'
import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import { useCaptureStore } from '@/stores/capture'
import { useViewStore } from '@/stores/view'

import SegmentMeasure from './SegmentMeasure.vue'

vi.mock('@/ipc/backend', () => ({
  getBackend: () => Promise.resolve({}),
}))

function summary(samplerate: number): CaptureSummary {
  return {
    id: 1,
    source: { kind: 'demo' },
    info: {
      samplerate,
      sampleCount: 10_000,
      triggerSample: null,
      triggerTimeMs: null,
      channels: [0],
      analog: [],
    },
    channels: [{ index: 0, name: '0' }],
    analog: [],
    decoders: null,
    session: null,
  }
}

describe('segment in the status bar', () => {
  beforeEach(() => {
    i18n.global.locale.value = 'en-US'
    setActivePinia(createPinia())
  })

  it('reads the length to one sample', async () => {
    const capture = useCaptureStore()
    const view = useViewStore()
    capture.summary = summary(400_000_000)
    await nextTick()
    const wrapper = mount(SegmentMeasure, { global: { plugins: [i18n] } })
    view.setSegment({ from: 100, to: 101 })
    await nextTick()
    expect(wrapper.find('button').text()).toBe('Segment 2.5 ns')
    view.setSegment({ from: 100, to: 1144 })
    await nextTick()
    expect(wrapper.find('button').text()).toBe('Segment 2.610 µs')
  })
})
