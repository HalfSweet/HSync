import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import type { DeviceState } from '@/ipc/bindings/DeviceState'
import { SESSION_FORMAT, SESSION_VERSION, type Session } from '@/session/session'

import { useCaptureStore } from './capture'
import { useDeviceStore } from './device'
import { useSessionStore } from './session'

const files = new Map<string, string>()

vi.mock('@/ipc/backend', () => ({
  getBackend: () =>
    Promise.resolve({
      decodeList: () => Promise.resolve([]),
      decodeView: () => Promise.resolve([]),
      readText: (path: string) => Promise.resolve(files.get(path) ?? ''),
    }),
}))

const I2C = [{ id: '1:i2c', channel: [{ scl: 1 }, { sda: 0 }] }]
const UART = [{ id: '1:uart', channel: [{ rxtx: 5 }] }]

function stateOf(driver: string): DeviceState {
  return {
    info: {
      id: driver,
      driver,
      vendor: 'HSync',
      model: driver,
      firmware: null,
      usbSpeed: null,
      slowPort: false,
    },
    options: [],
    channelCount: 16,
    enabledChannels: [0, 1],
    capturing: false,
    trigger: { maxPosition: 90, advanced: false },
    canLoop: false,
    hardwareDepth: null,
  }
}

function session(patch: Partial<Session>): Session {
  return {
    format: SESSION_FORMAT,
    version: SESSION_VERSION,
    device: null,
    options: [],
    enabledChannels: null,
    channelNames: {},
    colours: {},
    triggers: {},
    trigger: null,
    decoders: null,
    captureMode: null,
    ...patch,
  }
}

let lastId = 0
function captured(source: CaptureSummary['source'], names: string[], decoders: unknown) {
  lastId += 1
  return {
    id: lastId,
    source,
    info: {
      samplerate: 1_000_000,
      sampleCount: 1024,
      triggerSample: null,
      triggerTimeMs: null,
      channels: names.map((_, index) => index),
    },
    channels: names.map((name, index) => ({ index, name })),
    decoders,
  } satisfies CaptureSummary
}

const remembered = (driver: string) =>
  JSON.parse(localStorage.getItem(`hsync.session.${driver}`) ?? 'null') as Session | null
const settle = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('session store', () => {
  beforeEach(() => {
    localStorage.clear()
    files.clear()
    setActivePinia(createPinia())
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('keeps the names and decoders remembered for a device for its first capture', async () => {
    localStorage.setItem(
      'hsync.session.DSLogic',
      JSON.stringify(session({ channelNames: { 0: 'SDA' }, decoders: I2C })),
    )
    // Remembered before names and decoders were: the device's own apply.
    localStorage.setItem('hsync.session.virtual-demo', JSON.stringify(session({})))
    useSessionStore()
    const device = useDeviceStore()
    const capture = useCaptureStore()

    device.state = stateOf('DSLogic')
    await settle()
    expect(capture.preset).toEqual({ channelNames: { 0: 'SDA' }, decoders: I2C })

    device.state = stateOf('virtual-demo')
    await settle()
    expect(capture.preset).toBeNull()
  })

  it("remembers the names and decoders of the device's own captures", async () => {
    vi.useFakeTimers()
    useSessionStore()
    const device = useDeviceStore()
    const capture = useCaptureStore()
    const dslogic = { kind: 'device', driver: 'DSLogic', model: 'DSLogic' } as const

    device.state = stateOf('DSLogic')
    capture.summary = captured(dslogic, ['SDA', '1'], I2C)
    await vi.advanceTimersByTimeAsync(600)
    expect(remembered('DSLogic')?.channelNames).toEqual({ 0: 'SDA' })
    expect(remembered('DSLogic')?.decoders).toEqual(I2C)

    // A file's names and decoders are not the device's.
    capture.summary = captured({ kind: 'file', path: '/a.dsl' }, ['RX'], UART)
    await vi.advanceTimersByTimeAsync(600)
    expect(remembered('DSLogic')?.decoders).toEqual(I2C)

    // Taking the decoders away is remembered too.
    capture.summary = captured(dslogic, ['0', '1'], null)
    await vi.advanceTimersByTimeAsync(600)
    expect(remembered('DSLogic')?.channelNames).toEqual({})
    expect(remembered('DSLogic')?.decoders).toEqual([])
  })

  it('gives a session loaded before any capture to the first capture', async () => {
    const sessions = useSessionStore()
    const capture = useCaptureStore()
    capture.preset = { channelNames: {}, decoders: I2C }

    files.set('names.json', JSON.stringify(session({ channelNames: { 5: 'RX' } })))
    expect(await sessions.load('names.json')).toBeNull()
    // Without decoders of its own the session leaves those there are.
    expect(capture.preset).toEqual({ channelNames: { 5: 'RX' }, decoders: I2C })

    files.set('uart.json', JSON.stringify(session({ decoders: UART })))
    await sessions.load('uart.json')
    expect(capture.preset).toEqual({ channelNames: {}, decoders: UART })
    expect(sessions.snapshot().decoders).toEqual(UART)
  })
})
