import { flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { activeCatalog, DEFAULT_CATALOG } from '@/decode/catalog'
import type { CatalogEntry } from '@/ipc/bindings/CatalogEntry'
import type { ModuleInfo } from '@/ipc/bindings/ModuleInfo'

import { useDecodeStore } from './decode'

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

describe('the decode store', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
    activeCatalog.value = DEFAULT_CATALOG
    backend.decodeModules.mockResolvedValue([])
    backend.decodeCatalog.mockResolvedValue([])
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
})
