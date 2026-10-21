import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ref, watch } from 'vue'

import { onFrameDrawn, onNextFrame } from './frames'

/** Runs the next animation frame and what it awaits. */
async function frame(): Promise<void> {
  vi.advanceTimersToNextFrame()
  for (let i = 0; i < 10; i++) await Promise.resolve()
}

describe('frames', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['requestAnimationFrame', 'cancelAnimationFrame', 'performance'] })
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  it('draws what a frame moved in that frame, once Vue has applied it', async () => {
    const position = ref(0)
    const drawn: string[] = []
    const stop = watch(position, () =>
      onFrameDrawn((now) => drawn.push(`draw ${position.value} at ${now}`)),
    )
    let moved = 0
    onNextFrame((now) => {
      moved = now
      position.value = 1
    })
    await frame()
    expect(drawn).toEqual([`draw 1 at ${moved}`])
    stop()
  })

  it('runs what a frame asks for next in the following frame', async () => {
    const times: number[] = []
    onNextFrame((now) => {
      times.push(now)
      onNextFrame((later) => times.push(later))
    })
    await frame()
    expect(times).toHaveLength(1)
    await frame()
    expect(times).toHaveLength(2)
    expect(times[1]).toBeGreaterThan(times[0] ?? Infinity)
  })

  it('draws in the next frame when asked outside one', async () => {
    const drawn: number[] = []
    onFrameDrawn((now) => drawn.push(now))
    expect(drawn).toEqual([])
    await frame()
    expect(drawn).toHaveLength(1)
  })
})
