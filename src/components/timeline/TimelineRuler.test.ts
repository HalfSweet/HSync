import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import { i18n } from '@/i18n'
import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import { useCaptureStore } from '@/stores/capture'
import { useViewStore } from '@/stores/view'

import TimelineRuler from './TimelineRuler.vue'

vi.mock('@/ipc/backend', () => ({
  getBackend: () => Promise.resolve({}),
}))

function summary(samplerate: number, sampleCount: number): CaptureSummary {
  return {
    id: 1,
    source: { kind: 'demo' },
    info: {
      samplerate,
      sampleCount,
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

/** A ruler 1000 px wide over samples 0–1000 of a capture at `samplerate`: one sample a pixel. */
async function ruler(samplerate = 100_000_000) {
  setActivePinia(createPinia())
  const capture = useCaptureStore()
  const view = useViewStore()
  capture.summary = summary(samplerate, 10_000_000)
  await nextTick()
  view.setWidth(1000)
  view.showRange(0, 1000)
  const wrapper = mount(TimelineRuler, { global: { plugins: [i18n] } })
  const track = wrapper.find('[title="Drag along the ruler to select a segment"]')
    .element as HTMLElement
  // jsdom has neither layout nor pointer capture.
  track.getBoundingClientRect = () => ({ left: 0, top: 0, width: 1000, height: 40 }) as DOMRect
  track.setPointerCapture = vi.fn()
  return { wrapper, view, track }
}

/** Presses at `from`, moves through `path` and lets go at its last point (track pixels). */
async function drag(track: HTMLElement, from: number, ...path: number[]): Promise<void> {
  const send = (type: string, clientX: number) =>
    track.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerId: 1, button: 0, clientX }))
  send('pointerdown', from)
  for (const x of path) send('pointermove', x)
  send('pointerup', path.at(-1) ?? from)
  await nextTick()
}

describe('timeline ruler', () => {
  beforeEach(() => {
    i18n.global.locale.value = 'en-US'
  })

  it('keeps a segment shorter than 2 µs when zoomed in', async () => {
    const { wrapper, view, track } = await ruler()
    // 50 px at 10 ns a pixel: 500 ns.
    await drag(track, 100, 120, 150)
    expect(view.segment).toEqual({ from: 100, to: 150 })
    expect(wrapper.text()).toContain('500 ns')
  })

  it('marks a segment either way along the ruler', async () => {
    const { view, track } = await ruler()
    await drag(track, 400, 380, 300)
    expect(view.segment).toEqual({ from: 300, to: 400 })
  })

  it('takes a press that moves 3 px or less for a click, which clears the segment', async () => {
    const { view, track } = await ruler()
    view.setSegment({ from: 100, to: 600 })
    await drag(track, 300, 302, 303)
    expect(view.segment).toBeNull()
  })

  it('leaves the segment alone until the pointer has moved more than 3 px', async () => {
    const { view, track } = await ruler()
    view.setSegment({ from: 100, to: 600 })
    track.dispatchEvent(
      new PointerEvent('pointerdown', { bubbles: true, pointerId: 1, button: 0, clientX: 300 }),
    )
    track.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, clientX: 302 }))
    expect(view.segment).toEqual({ from: 100, to: 600 })
    track.dispatchEvent(new PointerEvent('pointermove', { bubbles: true, clientX: 310 }))
    expect(view.segment).toEqual({ from: 300, to: 310 })
  })
})
