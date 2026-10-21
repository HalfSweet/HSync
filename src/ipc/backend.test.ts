import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { getBackend } from './backend'

vi.mock(import('@tauri-apps/api/core'), async (importOriginal) => ({
  ...(await importOriginal()),
  isTauri: () => true,
  invoke: vi.fn(),
}))

describe('Tauri decode coverage', () => {
  beforeEach(() => vi.clearAllMocks())

  it.each([
    [144077.46278317153, 144078],
    [0.25, 1],
    [0, 1],
    [64, 64],
  ])('sends %s samples per pixel as integer resolution %s', async (resolution, expected) => {
    const coverage = [[10, 20]]
    vi.mocked(invoke).mockResolvedValueOnce(coverage)
    const backend = await getBackend()

    await expect(backend.decodeCoverage(1, 2, 0, 200_000, resolution)).resolves.toEqual(coverage)
    expect(invoke).toHaveBeenCalledExactlyOnceWith('decode_coverage', {
      captureId: 1,
      index: 2,
      start: 0,
      end: 200_000,
      resolution: expected,
    })
  })
})
