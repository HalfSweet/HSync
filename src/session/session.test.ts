import { describe, expect, it } from 'vitest'

import { optionRank, parseSession } from './session'

describe('sessions', () => {
  it('read HSync sessions back', () => {
    const session = parseSession(
      JSON.stringify({
        format: 'hsync-session',
        version: 1,
        device: { driver: 'DSLogic', model: 'DSLogic U2Pro16' },
        options: [
          ['samplerate', { type: 'integer', value: 25_000_000 }],
          ['bogus', { type: 'integer', value: 1 }],
          ['filter', { type: 'bool', value: 'yes' }],
        ],
        enabledChannels: [0, 1],
        channelNames: { 0: 'SDA' },
        colours: { 0: 5 },
        triggers: { 1: 'fall', 2: 'sideways' },
        trigger: null,
        decoders: [{ id: '1:i2c' }],
      }),
    )
    expect(session?.device).toEqual({ driver: 'DSLogic', model: 'DSLogic U2Pro16' })
    expect(session?.options).toEqual([['samplerate', { type: 'integer', value: 25_000_000 }]])
    expect(session?.triggers).toEqual({ 1: 'fall' })
    expect(session?.channelNames).toEqual({ 0: 'SDA' })
    expect(session?.decoders).toEqual([{ id: '1:i2c' }])
  })

  it('read what they can of DSView sessions', () => {
    const session = parseSession(
      JSON.stringify({
        Version: 3,
        Device: 'DSLogic',
        'Sample rate': '1000000',
        'Sample count': '1000448',
        'Operation Mode': 1,
        channel: [
          { index: 0, name: '0', enabled: true, strigger: 3, type: 10000 },
          { index: 1, name: 'SCL', enabled: false, strigger: 0, type: 10000 },
        ],
        decoder: [],
        trigger: { triggerPos: 25 },
      }),
    )
    expect(session?.options).toEqual([
      ['samplerate', { type: 'integer', value: 1_000_000 }],
      ['sampleLimit', { type: 'integer', value: 1_000_448 }],
    ])
    expect(session?.enabledChannels).toEqual([0])
    expect(session?.channelNames).toEqual({ 1: 'SCL' })
    expect(session?.triggers).toEqual({ 0: 'fall' })
    expect(session?.trigger?.position).toBe(25)
    expect(parseSession('{"nothing": 1}')).toBeNull()
    expect(parseSession('not json')).toBeNull()
  })

  it('apply modes before the settings they decide', () => {
    expect(optionRank('operationMode')).toBeLessThan(optionRank('samplerate'))
    expect(optionRank('channelMode')).toBeLessThan(optionRank('sampleLimit'))
  })
})
