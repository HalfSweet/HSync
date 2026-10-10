import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import type { AvailableUpdate, UpdateProgress } from '@/ipc/backend'

import { useUpdateStore } from './update'

const backend = {
  updateCheck: vi.fn<() => Promise<AvailableUpdate | null>>(),
  updateInstall: vi.fn<(onProgress: (progress: UpdateProgress) => void) => Promise<void>>(),
}
vi.mock('@/ipc/backend', () => ({ getBackend: () => Promise.resolve(backend) }))

const failure = Object.assign(new Error('offline'), {
  code: 'error.update.check',
  params: { detail: 'offline' },
})

describe('update store', () => {
  beforeEach(() => {
    localStorage.clear()
    setActivePinia(createPinia())
    backend.updateCheck.mockReset()
    backend.updateInstall.mockReset()
  })

  it('finds a newer release or tells this is the latest', async () => {
    const update = useUpdateStore()
    backend.updateCheck.mockResolvedValueOnce({ version: '0.3.0' })
    await update.check()
    expect(update.state).toBe('available')
    expect(update.available).toEqual({ version: '0.3.0' })

    backend.updateCheck.mockResolvedValueOnce(null)
    await update.check()
    expect(update.state).toBe('latest')
    expect(update.available).toBeNull()
  })

  it('shows a failed check unless it was the quiet one at startup', async () => {
    const update = useUpdateStore()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    backend.updateCheck.mockRejectedValue(failure)
    await update.check(true)
    expect(update.state).toBe('idle')
    expect(update.error).toBeNull()

    await update.check()
    expect(update.state).toBe('failed')
    expect(update.error).toMatchObject({
      code: 'error.update.check',
      params: { detail: 'offline' },
    })
  })

  it('reports the download and then restarts', async () => {
    const update = useUpdateStore()
    backend.updateCheck.mockResolvedValueOnce({ version: '0.3.0' })
    await update.check()
    const seen: (UpdateProgress | null)[] = []
    backend.updateInstall.mockImplementationOnce((onProgress) => {
      onProgress({ done: 0, total: 200 })
      seen.push(update.progress)
      onProgress({ done: 50, total: 200 })
      seen.push(update.progress)
      expect(update.state).toBe('downloading')
      return Promise.resolve()
    })
    await update.install()
    expect(seen).toEqual([
      { done: 0, total: 200 },
      { done: 50, total: 200 },
    ])
    expect(update.state).toBe('restarting')
  })

  it('keeps the release offered when installing it fails', async () => {
    const update = useUpdateStore()
    backend.updateCheck.mockResolvedValueOnce({ version: '0.3.0' })
    await update.check()
    backend.updateInstall.mockRejectedValueOnce({
      code: 'error.update.install',
      params: { detail: 'signature' },
    })
    await update.install()
    expect(update.state).toBe('failed')
    expect(update.available).toEqual({ version: '0.3.0' })
    expect(update.error?.code).toBe('error.update.install')
  })

  it('checks at startup only when the settings allow it, and never in development', async () => {
    backend.updateCheck.mockResolvedValue(null)
    await useUpdateStore().init()
    // Vitest runs as a development build.
    expect(backend.updateCheck).not.toHaveBeenCalled()

    vi.stubEnv('DEV', false)
    try {
      setActivePinia(createPinia())
      await useUpdateStore().init()
      expect(backend.updateCheck).toHaveBeenCalledTimes(1)

      localStorage.setItem('hsync.checkUpdates', 'false')
      setActivePinia(createPinia())
      await useUpdateStore().init()
      expect(backend.updateCheck).toHaveBeenCalledTimes(1)
    } finally {
      vi.unstubAllEnvs()
    }
  })
})
