import { createPinia, setActivePinia } from 'pinia'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import type { DeviceState } from '@/ipc/bindings/DeviceState'
import type { OptionValue } from '@/ipc/bindings/OptionValue'
import { uiError } from '@/ipc/mock/mockDevice'
import { SESSION_FORMAT, SESSION_VERSION, type Session } from '@/session/session'

import { useCaptureStore } from './capture'
import { useChannelsStore } from './channels'
import { useDeviceStore } from './device'
import { useSessionStore } from './session'

const files = new Map<string, string>()

/** A device that, as an ATK-Logic in Stream mode, runs 100 MHz with three channels at most. */
const SLOW = 20_000_000
const FAST = 100_000_000
let atk: DeviceState

function atkState(enabledChannels: number[], rate: number): DeviceState {
  const fast = enabledChannels.length <= 3
  return {
    ...stateOf('ATK-Logic'),
    options: [
      {
        key: 'samplerate',
        kind: { type: 'integers', values: fast ? [SLOW, FAST] : [SLOW] },
        value: { type: 'integer', value: fast ? rate : SLOW },
        enabled: true,
      },
    ],
    enabledChannels,
  }
}

function setRate(value: OptionValue): Promise<DeviceState> {
  if (value.value === FAST && atk.enabledChannels.length > 3)
    return Promise.reject(uiError('error.device.invalidRequest'))
  atk = atkState(atk.enabledChannels, Number(value.value))
  return Promise.resolve(atk)
}

function setChannel(channel: number, enabled: boolean): Promise<DeviceState> {
  const others = atk.enabledChannels.filter((index) => index !== channel)
  const rate = atk.options[0]?.value.value
  atk = atkState(enabled ? [...others, channel].sort((a, b) => a - b) : others, Number(rate))
  return Promise.resolve(atk)
}

vi.mock('@/ipc/backend', () => ({
  getBackend: () =>
    Promise.resolve({
      decodeList: () => Promise.resolve([]),
      decodeView: () => Promise.resolve([]),
      readText: (path: string) => Promise.resolve(files.get(path) ?? ''),
      deviceSetOption: (_key: string, value: OptionValue) => setRate(value),
      deviceSetChannel: setChannel,
    }),
}))

const I2C = [{ nodes: { i2c: { type: 'i2c', config: { scl: 1, sda: 0 } } } }]
const UART = [{ nodes: { uart: { type: 'uart', config: { line: 5 } } } }]

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
    trigger: { maxPosition: 90, advanced: false, advancedOtherMode: false },
    canLoop: false,
    hardwareDepth: null,
    rateTiers: [],
    rateTier: null,
    analogInputs: [],
  }
}

function session(patch: Partial<Session>): Session {
  return {
    format: SESSION_FORMAT,
    version: SESSION_VERSION,
    device: null,
    options: [],
    enabledChannels: null,
    analogInputs: null,
    channelNames: {},
    colours: {},
    heights: {},
    triggers: {},
    trigger: null,
    decoders: null,
    captureMode: null,
    view: null,
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
      analog: [],
    },
    channels: names.map((name, index) => ({ index, name })),
    analog: [],
    decoders,
    session: null,
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

  it('sets the rates a device allows only once its channels are set', async () => {
    localStorage.setItem(
      'hsync.session.ATK-Logic',
      JSON.stringify(
        session({
          options: [['samplerate', { type: 'integer', value: FAST }]],
          enabledChannels: [0, 1, 2],
        }),
      ),
    )
    useSessionStore()
    const device = useDeviceStore()

    atk = atkState([...Array(16).keys()], SLOW)
    device.state = atk
    await settle()
    expect(device.enabledChannels).toEqual([0, 1, 2])
    expect(device.samplerate).toBe(FAST)
    expect(device.error).toBeNull()
  })

  it('keeps the channel colours and heights with a capture and brings them back', async () => {
    const sessions = useSessionStore()
    const channels = useChannelsStore()
    channels.setColour(2, 6)
    channels.setHeight(2, 72)
    const document: unknown = JSON.parse(JSON.stringify(sessions.captureDocument()))
    channels.colours = {}
    channels.heights = {}
    await sessions.applyCaptureDocument(document)
    expect(channels.colours).toEqual({ 2: 6 })
    expect(channels.heights).toEqual({ 2: 72 })
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

it('keeps unsupported DSView configurations in the first-capture preset and session snapshot', async () => {
  setActivePinia(createPinia())
  const sessions = useSessionStore()
  const capture = useCaptureStore()
  const raw = [
    { id: 'future-bus', options: { bytes: [0, 255] }, 'stacked decoders': [{ id: 'future-chip' }] },
  ]
  files.set('retained.dsc', JSON.stringify({ Version: 3, Device: 'DSLogic', decoder: raw }))
  await sessions.load('retained.dsc')
  expect(capture.preset?.decoders).toEqual(raw)
  expect(sessions.snapshot().decoders).toEqual(raw)
})
