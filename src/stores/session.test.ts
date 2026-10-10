import { createPinia, setActivePinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { SESSION_FORMAT, SESSION_VERSION, type Session } from '@/session/session'

import { useCaptureStore } from './capture'
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

describe('session store', () => {
  beforeEach(() => {
    localStorage.clear()
    files.clear()
    setActivePinia(createPinia())
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
