import { flushPromises } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, nextTick } from 'vue'

import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import { useCaptureStore } from '@/stores/capture'
import { useSearchStore } from '@/stores/search'
import { useStreamStore } from '@/stores/stream'
import { useViewStore } from '@/stores/view'

import type { ToMain, ToStream } from './messages'
import { useStreamWindowGuest } from './streamGuest'
import { useStreamWindowHost } from './streamHost'

/** The two windows' link, as each side sees it in these tests. */
const link = vi.hoisted(() => ({
  toStream: [] as unknown[],
  toMain: [] as unknown[],
  mainHandler: null as ((message: never) => void) | null,
  streamHandler: null as ((message: never) => void) | null,
  closedHandler: null as (() => void) | null,
  opened: 0,
  closed: 0,
}))
vi.mock('@/ipc/windowLink', () => ({
  tellStream: (message: unknown) => {
    link.toStream.push(message)
    return Promise.resolve()
  },
  tellMain: (message: unknown) => {
    link.toMain.push(message)
    return Promise.resolve()
  },
  onMainMessage: (handler: (message: never) => void) => {
    link.mainHandler = handler
    return Promise.resolve(() => (link.mainHandler = null))
  },
  onStreamMessage: (handler: (message: never) => void) => {
    link.streamHandler = handler
    return Promise.resolve(() => (link.streamHandler = null))
  },
  onStreamWindowClosed: (handler: () => void) => {
    link.closedHandler = handler
    return Promise.resolve(() => (link.closedHandler = null))
  },
  openStreamWindow: () => {
    link.opened += 1
    return Promise.resolve()
  },
  closeStreamWindow: () => {
    link.closed += 1
    return Promise.resolve()
  },
}))
vi.mock('@/ipc/backend', () => ({
  getBackend: () =>
    Promise.resolve(
      new Proxy(
        {},
        {
          get: (_, name: string) =>
            name === 'then'
              ? undefined
              : name.startsWith('on')
                ? () => Promise.resolve(() => undefined)
                : ['decodeList', 'decodeCatalog', 'decodeModules'].includes(name)
                  ? () => Promise.resolve([])
                  : () => Promise.resolve(null),
        },
      ),
    ),
}))

const summary: CaptureSummary = {
  id: 4,
  source: { kind: 'demo' },
  info: {
    samplerate: 1e6,
    sampleCount: 1000,
    triggerSample: null,
    triggerTimeMs: null,
    channels: [0],
  },
  channels: [{ index: 0, name: '0' }],
  decoders: null,
  session: null,
}

const toMain = (message: ToMain) => link.mainHandler?.(message as never)
const toStream = (message: ToStream) => link.streamHandler?.(message as never)

function inScope(setup: () => void) {
  const scope = effectScope()
  scope.run(setup)
  return scope
}

beforeEach(() => {
  setActivePinia(createPinia())
  localStorage.clear()
  Object.assign(link, { toStream: [], toMain: [], opened: 0, closed: 0 })
})

describe('data stream modes', () => {
  it('moves into a window of its own, remembered, and back beside the timeline', async () => {
    const stream = useStreamStore()
    stream.open = true
    stream.detach()
    expect([stream.detached, stream.open, stream.windowOpen, stream.shown]).toEqual([
      true,
      false,
      true,
      true,
    ])
    expect(link.opened).toBe(1)
    await nextTick()
    expect(localStorage.getItem('hsync.stream.detached')).toBe('true')

    // The top bar's button brings the window back rather than opening the panel.
    stream.windowOpen = false
    stream.toggle()
    expect([stream.open, stream.windowOpen, link.opened]).toEqual([false, true, 2])
    // A group header's button lists its decoder there.
    stream.show(2, 1)
    expect([stream.decoder, stream.stream, link.opened]).toEqual([2, 1, 3])

    stream.dock()
    expect([stream.detached, stream.open, stream.windowOpen, link.closed]).toEqual([
      false,
      true,
      false,
      1,
    ])
  })
})

describe('the main window with the data stream window', () => {
  it('tells the window what it shows while it is open, and does what it asks', async () => {
    const capture = useCaptureStore()
    const stream = useStreamStore()
    const view = useViewStore()
    const scope = inScope(useStreamWindowHost)
    await flushPromises()

    capture.summary = summary
    await nextTick()
    expect(link.toStream).toEqual([])

    stream.windowOpen = true
    toMain({ type: 'hello' })
    expect(link.toStream.map((message) => (message as ToStream).type)).toEqual([
      'capture',
      'segment',
      'found',
      'listing',
    ])

    view.setSegment({ from: 10, to: 20 })
    await nextTick()
    expect(link.toStream.at(-1)).toEqual({ type: 'segment', segment: { from: 10, to: 20 } })

    toMain({ type: 'reveal', span: { start: 400, end: 600 } })
    expect(view.segment).toEqual({ from: 400, to: 600 })
    toMain({ type: 'hot', span: { start: 1, end: 2 } })
    expect(stream.hot).toEqual({ start: 1, end: 2 })
    toMain({ type: 'listing', listing: { decoder: 1, stream: 0 } })
    expect(stream.decoder).toBe(1)

    link.closedHandler?.()
    expect([stream.windowOpen, stream.hot]).toEqual([false, null])

    stream.detached = true
    toMain({ type: 'dock' })
    expect([stream.detached, stream.open]).toEqual([false, true])
    scope.stop()
  })
})

describe('the data stream window', () => {
  it('takes what the main window shows and hands it what happens here', async () => {
    const capture = useCaptureStore()
    const finder = useSearchStore()
    const stream = useStreamStore()
    const view = useViewStore()
    const scope = inScope(useStreamWindowGuest)
    await flushPromises()
    expect(stream.open).toBe(true)
    expect(link.toMain).toEqual([{ type: 'hello' }])

    toStream({ type: 'capture', summary, live: null })
    toStream({ type: 'segment', segment: { from: 5, to: 9, blockId: 'b' } })
    toStream({ type: 'found', result: { hits: [], truncated: false }, current: null })
    toStream({ type: 'current', current: 3 })
    toStream({ type: 'listing', listing: { decoder: 2, stream: 1 } })
    expect(capture.summary?.id).toBe(4)
    expect(view.segment).toEqual({ from: 5, to: 9, blockId: 'b' })
    expect([finder.result?.truncated, finder.current]).toEqual([false, 3])
    await nextTick()
    expect([stream.decoder, stream.stream]).toEqual([2, 1])
    expect(link.toMain.at(-1)).toEqual({ type: 'listing', listing: { decoder: 2, stream: 1 } })

    // Selecting an event marks it here at once, and the main window shows it.
    stream.reveal({ start: 40, end: 60 })
    expect(view.segment).toEqual({ from: 40, to: 60 })
    expect(link.toMain.at(-1)).toEqual({ type: 'reveal', span: { start: 40, end: 60 } })

    stream.hot = { start: 1, end: 3 }
    await nextTick()
    expect(link.toMain.at(-1)).toEqual({ type: 'hot', span: { start: 1, end: 3 } })
    scope.stop()
  })
})
