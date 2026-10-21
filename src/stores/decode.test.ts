import { flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { activeCatalog, DEFAULT_CATALOG } from '@/decode/catalog'
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'
import type { DecoderLanes } from '@/ipc/bindings/DecoderLanes'
import type { DecoderStatus } from '@/ipc/bindings/DecoderStatus'
import type { ModuleInfo } from '@/ipc/bindings/ModuleInfo'

import { useCaptureStore } from './capture'
import { useDecodeStore } from './decode'
import { useViewStore } from './view'

const sensor: ModuleInfo = {
  name: 'my-sensor',
  path: '/captures/sensor.hsync',
  category: 'line',
  shadows: false,
}
const sensorType: CatalogEntry = {
  ...(DEFAULT_CATALOG[0] as CatalogEntry),
  name: 'my-sensor',
  module: sensor.path,
}
let modulesChanged: () => void = () => undefined
const backend = {
  decodeList: vi.fn<() => Promise<DecoderStatus[]>>(() => Promise.resolve([])),
  decodeView: vi.fn<
    (
      captureId: number,
      start: number,
      end: number,
      samplesPerPixel: number,
    ) => Promise<DecoderLanes[]>
  >(() => Promise.resolve([])),
  decodeCatalog: vi.fn<() => Promise<CatalogEntry[]>>(() => Promise.resolve([])),
  decodeModules: vi.fn<() => Promise<ModuleInfo[]>>(() => Promise.resolve([])),
  decodeLoadModule: vi.fn<(path: string) => Promise<ModuleInfo>>((path) =>
    Promise.resolve({ ...sensor, path }),
  ),
  decodeUnloadModule: vi.fn(() => Promise.resolve(true)),
  onDecoded: vi.fn(() => Promise.resolve(() => undefined)),
  onDecodeProgress: vi.fn(() => Promise.resolve(() => undefined)),
  onModulesChanged: vi.fn((handler: () => void) => {
    modulesChanged = handler
    return Promise.resolve(() => undefined)
  }),
}
vi.mock('@/ipc/backend', () => ({ getBackend: () => Promise.resolve(backend) }))

function lane(start: number, truncated: boolean): DecoderLanes[] {
  return [
    {
      index: 0,
      streams: [
        {
          stream: 0,
          view: {
            truncated,
            blocks: [
              {
                type: 'event',
                index: 0,
                start,
                end: start + 10,
                kind: 'word',
                payload: { kind: 'word', value: 42 },
              },
            ],
          },
        },
      ],
    },
  ]
}

function captureWithViewport() {
  useCaptureStore().summary = {
    id: 1,
    source: { kind: 'demo' },
    info: {
      samplerate: 1_000_000,
      sampleCount: 1000,
      triggerSample: null,
      triggerTimeMs: null,
      channels: [0],
    },
    channels: [{ index: 0, name: '0' }],
    decoders: null,
    session: null,
  }
  const view = useViewStore()
  view.viewport = { start: 100, width: 100, samplesPerPixel: 1 }
  return view
}

describe('the decode store', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
    activeCatalog.value = DEFAULT_CATALOG
    backend.decodeModules.mockResolvedValue([])
    backend.decodeCatalog.mockResolvedValue([])
    backend.decodeView.mockReset().mockResolvedValue([])
  })

  it('lists the modules a capture file brought beside those loaded here', async () => {
    const decode = useDecodeStore()
    await decode.init()
    expect(decode.carried).toEqual([])

    // A file opened brings a module: the backend says so, and lists it.
    backend.decodeModules.mockResolvedValue([sensor])
    backend.decodeCatalog.mockResolvedValue([...DEFAULT_CATALOG, sensorType])
    modulesChanged()
    await flushPromises()
    expect(decode.carried).toEqual([sensor])
    expect(decode.catalog.some((type) => type.name === 'my-sensor')).toBe(true)

    // A module loaded here under that name is this one's, not the file's.
    const own = { ...sensor, path: '/decoders/my-sensor.json' }
    backend.decodeModules.mockResolvedValue([own])
    await decode.loadModule(own.path)
    expect(decode.modules.map((entry) => entry.info)).toEqual([own])
    expect(decode.carried).toEqual([])

    // Let go here, the file's is listed again once the backend holds it again.
    backend.decodeModules.mockResolvedValue([sensor])
    await decode.unloadModule(own.path)
    expect(backend.decodeUnloadModule).toHaveBeenCalledWith('my-sensor')
    expect(decode.modules).toEqual([])
    expect(decode.carried).toEqual([sensor])

    backend.decodeModules.mockResolvedValue([])
    await decode.unloadCarried('my-sensor')
    expect(backend.decodeUnloadModule).toHaveBeenCalledTimes(2)
    expect(decode.carried).toEqual([])
  })

  it('refetches the visible window when overscan exhausts the block limit, then refetches on pan', async () => {
    const view = captureWithViewport()
    backend.decodeView.mockResolvedValueOnce(lane(10, true)).mockResolvedValueOnce(lane(110, false))
    const decode = useDecodeStore()
    await flushPromises()

    expect(backend.decodeView.mock.calls).toEqual([
      [1, 0, 300, 1],
      [1, 100, 200, 1],
    ])
    expect(decode.streamView(0, 0)?.blocks[0]?.start).toBe(110)

    // This is inside the first request's overscan, but outside the complete fallback.
    backend.decodeView.mockResolvedValueOnce(lane(210, false))
    view.pan(50)
    await flushPromises()
    expect(backend.decodeView).toHaveBeenCalledTimes(3)
    expect(backend.decodeView).toHaveBeenLastCalledWith(1, 50, 350, 1)
    expect(decode.streamView(0, 0)?.blocks[0]?.start).toBe(210)
  })

  it('keeps the bounded result when the visible window itself exceeds the block limit', async () => {
    captureWithViewport()
    backend.decodeView.mockResolvedValue(lane(110, true))
    const decode = useDecodeStore()
    await flushPromises()

    expect(backend.decodeView).toHaveBeenCalledTimes(2)
    expect(decode.streamView(0, 0)?.truncated).toBe(true)
    await flushPromises()
    expect(backend.decodeView).toHaveBeenCalledTimes(2)
  })
})
