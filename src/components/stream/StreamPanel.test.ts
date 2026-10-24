import { flushPromises, mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'

import { i18n, setLocale } from '@/i18n'
import type { StreamRequest } from '@/ipc/backend'
import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import type { DecoderStatus } from '@/ipc/bindings/DecoderStatus'
import type { EventRow } from '@/ipc/bindings/EventRow'
import { useCaptureStore } from '@/stores/capture'
import { useDecodeStore } from '@/stores/decode'
import { useStreamStore } from '@/stores/stream'
import { useViewStore } from '@/stores/view'

import StreamPanel from './StreamPanel.vue'

const summary: CaptureSummary = {
  id: 3,
  source: { kind: 'device', driver: 'virtual-demo', model: 'Demo' },
  info: {
    samplerate: 1_000_000,
    sampleCount: 10_000,
    triggerSample: null,
    triggerTimeMs: null,
    channels: [0, 1],
  },
  channels: [
    { index: 0, name: '0' },
    { index: 1, name: '1' },
  ],
  decoders: null,
  session: null,
}

function decoder(index: number, kind: string): DecoderStatus {
  return {
    index,
    label: '',
    kinds: [kind],
    state: 'done',
    error: null,
    streams: [
      {
        index: 0,
        name: kind,
        kind,
        schema: `${kind}.events`,
        progress: { events: 3, settled: 10_000, ended: true },
      },
    ],
  }
}
const statuses = [decoder(0, 'i2c'), decoder(1, 'uart')]

const rows: EventRow[] = [
  {
    index: 0,
    start: 100,
    end: 300,
    kind: 'address',
    role: 'address',
    payload: { kind: 'address', tx: 1, address: 0x62, tenBit: false, read: true, ack: true },
  },
  {
    index: 1,
    start: 400,
    end: 600,
    kind: 'data',
    role: 'data',
    payload: { kind: 'data', tx: 1, byte: 0x4f, ack: false },
  },
  {
    index: 2,
    start: 700,
    end: 700,
    kind: 'end',
    role: 'framing',
    payload: { kind: 'end', tx: 1 },
  },
]

const backend = {
  decodeList: vi.fn(() => Promise.resolve(statuses)),
  decodeStream: vi.fn((request: StreamRequest) =>
    Promise.resolve({
      total: rows.length,
      events: rows.length,
      inside: null,
      rows: rows.slice(request.offset, request.offset + request.count),
    }),
  ),
  decodeRowAt: vi.fn((_id: number, _index: number, _stream: number, sample: number) =>
    Promise.resolve(rows.findIndex((row) => row.end > sample)),
  ),
}
// Whatever else the stores ask of the backend finds nothing; it is no promise itself.
const quiet = new Proxy(backend, {
  get: (target, name: string) =>
    name in target
      ? target[name as keyof typeof target]
      : name === 'then'
        ? undefined
        : name.startsWith('on')
          ? () => Promise.resolve(() => undefined)
          : () => Promise.resolve(null),
})
vi.mock('@/ipc/backend', () => ({ getBackend: () => Promise.resolve(quiet) }))

async function panel() {
  const capture = useCaptureStore()
  const decode = useDecodeStore()
  capture.summary = summary
  decode.statuses = statuses
  useStreamStore().show(0)
  const wrapper = mount(StreamPanel, { global: { plugins: [i18n] }, attachTo: document.body })
  await flushPromises()
  return wrapper
}

/** The listed rows, in order. */
const listed = (wrapper: Awaited<ReturnType<typeof panel>>) =>
  wrapper.findAll('.cursor-pointer').filter((row) => row.attributes('style')?.includes('top'))

describe('data stream panel', () => {
  // jsdom does not scroll.
  beforeAll(() => {
    Object.defineProperty(Element.prototype, 'scrollTo', { value: () => undefined })
  })

  beforeEach(() => {
    setActivePinia(createPinia())
    setLocale('en-US')
    document.body.innerHTML = ''
  })

  it('switches between the streams from a menu', async () => {
    const wrapper = await panel()
    const button = wrapper.get('[aria-haspopup="menu"]')
    expect(button.text()).toBe('I²C')
    await button.trigger('click')
    const items = wrapper.findAll('[role="menuitemradio"]')
    expect(items.map((item) => [item.text(), item.attributes('aria-checked')])).toEqual([
      ['I²C', 'true'],
      ['UART', 'false'],
    ])
    await items[1]?.trigger('click')
    expect(useStreamStore().decoder).toBe(1)
    expect(wrapper.find('[role="menu"]').exists()).toBe(false)
    wrapper.unmount()
  })

  it('puts the transfer number in a column of its own', async () => {
    const wrapper = await panel()
    const first = listed(wrapper)[0]
    const spans = first?.findAll(':scope > span').map((span) => span.text())
    expect(spans?.slice(0, 3)).toEqual(['0.1000', '1', 'Address'])
    // The address needs no label beside an address; labels and values differ in shade.
    const cells = first?.findAll('.cell') ?? []
    expect(cells.map((cell) => cell.text())).toEqual([
      '62',
      '10-bit address no',
      'Read yes',
      'ACK yes',
    ])
    expect(cells[1]?.get('span').classes()).toContain('text-ink-3')
    expect(wrapper.find('[title="Transaction"]').text()).toBe('#')
    wrapper.unmount()
  })

  it('shows every field of the event clicked, until the segment moves on', async () => {
    const wrapper = await panel()
    expect(wrapper.find('section').exists()).toBe(false)
    await listed(wrapper)[0]?.trigger('click')
    expect(useViewStore().segment).toEqual({ from: 100, to: 300 })
    const details = wrapper.get('section')
    expect(details.text()).toContain('Address0.1000 msLasts 200.0 µs')
    expect(details.findAll('dt').map((term) => term.text())).toEqual([
      'Transaction',
      'Address',
      '10-bit address',
      'Read',
      'ACK',
    ])
    expect(details.findAll('dd').map((value) => value.text())).toEqual([
      '1',
      '62',
      'no',
      'yes',
      'yes',
    ])
    // The rows stay listed while the segment's count comes.
    expect(listed(wrapper)).toHaveLength(rows.length)
    expect(listed(wrapper)[0]?.attributes('aria-current')).toBe('true')

    useViewStore().setSegment({ from: 0, to: 50 })
    await flushPromises()
    expect(wrapper.find('section').exists()).toBe(false)
    wrapper.unmount()
  })

  it('selects the event of a block clicked on the timeline', async () => {
    const wrapper = await panel()
    useViewStore().setSegment({ from: 400, to: 600, blockId: 'i2c:1' })
    await flushPromises()
    expect(wrapper.get('section').text()).toContain('Data0.4000 ms')
    wrapper.unmount()
  })

  it('steps through the events with the arrow keys', async () => {
    const wrapper = await panel()
    const list = wrapper.get('[tabindex="0"]')
    await list.trigger('keydown', { key: 'ArrowDown' })
    expect(useViewStore().segment).toEqual({ from: 100, to: 300 })
    await list.trigger('keydown', { key: 'ArrowDown' })
    await list.trigger('keydown', { key: 'ArrowDown' })
    expect(useViewStore().segment).toEqual({ from: 700, to: 700 })
    // An event of no length reads without one, and the last stays selected.
    expect(wrapper.get('section').text()).not.toContain('Lasts')
    await list.trigger('keydown', { key: 'ArrowDown' })
    expect(useViewStore().segment).toEqual({ from: 700, to: 700 })
    await list.trigger('keydown', { key: 'ArrowUp' })
    expect(useViewStore().segment).toEqual({ from: 400, to: 600 })
    wrapper.unmount()
  })
})
