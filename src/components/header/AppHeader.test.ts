import { flushPromises, shallowMount } from '@vue/test-utils'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { i18n } from '@/i18n'
import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import { pickSavePath } from '@/ipc/dialogs'
import { APPLE } from '@/shortcuts/chords'
import { useCaptureStore } from '@/stores/capture'
import { installWebviewGuards } from '@/windows/webview'

import AppHeader from './AppHeader.vue'

const backend = vi.hoisted(() => ({
  saveFile:
    vi.fn<
      (
        id: number,
        path: string,
        session: unknown,
        onProgress: (done: number) => void,
      ) => Promise<CaptureSummary>
    >(),
  decodeList: () => Promise.resolve([]),
  decodeView: () => Promise.resolve([]),
}))
vi.mock('@/ipc/backend', () => ({ getBackend: () => Promise.resolve(backend) }))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true }))
vi.mock('@/ipc/dialogs', () => ({
  pickFile: vi.fn(() => Promise.resolve(null)),
  pickSavePath: vi.fn<() => Promise<string | null>>(),
}))

const summary: CaptureSummary = {
  id: 7,
  source: { kind: 'file', path: '/captures/input.dsl' },
  info: {
    samplerate: 1_000_000,
    sampleCount: 4096,
    triggerSample: null,
    triggerTimeMs: null,
    channels: [0],
    analog: [],
  },
  channels: [{ index: 0, name: 'CLK' }],
  analog: [],
  decoders: null,
  session: null,
}

function saveShortcut(target: EventTarget = document.body) {
  const event = new KeyboardEvent('keydown', {
    key: 's',
    code: 'KeyS',
    ctrlKey: !APPLE,
    metaKey: APPLE,
    bubbles: true,
    cancelable: true,
  })
  target.dispatchEvent(event)
  return event
}

describe('the header save shortcut with desktop WebView guards', () => {
  let pinia: ReturnType<typeof createPinia>
  let wrapper: ReturnType<typeof shallowMount<typeof AppHeader>>
  let removeGuards: () => void

  beforeEach(() => {
    localStorage.clear()
    pinia = createPinia()
    setActivePinia(pinia)
    vi.clearAllMocks()
    vi.mocked(pickSavePath).mockResolvedValue('/tmp/saved-waveform')
    backend.saveFile.mockResolvedValue({
      ...summary,
      source: { kind: 'file', path: '/tmp/saved-waveform.hsync' },
    })
    useCaptureStore().summary = summary
    removeGuards = installWebviewGuards()
    wrapper = shallowMount(AppHeader, { attachTo: document.body, global: { plugins: [i18n] } })
  })

  afterEach(() => {
    wrapper.unmount()
    removeGuards()
    disposePinia(pinia)
  })

  it.each(['window', 'search field'])('saves the waveform from the %s', async (target) => {
    const event = saveShortcut(target === 'search field' ? wrapper.get('input').element : undefined)
    expect(event.defaultPrevented).toBe(true)
    await flushPromises()

    expect(pickSavePath).toHaveBeenCalledWith('/captures/input.hsync', [
      { name: expect.any(String), extensions: ['hsync'] },
    ])
    expect(backend.saveFile).toHaveBeenCalledExactlyOnceWith(
      summary.id,
      '/tmp/saved-waveform.hsync',
      expect.objectContaining({ channelNames: { 0: 'CLK' } }),
      expect.any(Function),
    )
    expect(useCaptureStore().savedPath).toBe('/tmp/saved-waveform.hsync')
  })

  it.each(['no capture', 'busy', 'cancelled dialog'])(
    'never falls back to saving the web page with %s',
    async (state) => {
      if (state === 'no capture') useCaptureStore().summary = null
      if (state === 'busy') useCaptureStore().job = { kind: 'export', progress: 0 }
      if (state === 'cancelled dialog') vi.mocked(pickSavePath).mockResolvedValue(null)

      expect(saveShortcut().defaultPrevented).toBe(true)
      await flushPromises()
      expect(backend.saveFile).not.toHaveBeenCalled()
      if (state !== 'cancelled dialog') expect(pickSavePath).not.toHaveBeenCalled()
    },
  )
})
