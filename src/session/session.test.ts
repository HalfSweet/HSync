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
    expect(session?.view).toBeNull()
  })

  it('keep the external trigger of PXLogic devices', () => {
    const session = parseSession(
      JSON.stringify({
        format: 'hsync-session',
        version: 1,
        device: { driver: 'PXLogic', model: 'PX-Logic 16 Plus' },
        options: [
          ['channelMode', { type: 'choice', value: '2' }],
          ['externalTrigger', { type: 'choice', value: 'fall' }],
        ],
      }),
    )
    expect(session?.options).toEqual([
      ['channelMode', { type: 'choice', value: '2' }],
      ['externalTrigger', { type: 'choice', value: 'fall' }],
    ])
    expect(optionRank('channelMode')).toBeLessThan(optionRank('externalTrigger'))
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
      ['operationMode', { type: 'choice', value: 'stream' }],
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

  it('read the DSLogic options of DSView sessions by code', () => {
    // DSView's DSLogic0.def.dsc.
    const session = parseSession(
      JSON.stringify({
        'Channel Mode': 0,
        CollectMode: 2,
        Device: 'DSLogic',
        DeviceMode: 0,
        'Enable RLE Compress': 0,
        'Filter Targets': 1,
        'Max Height': '1X',
        'Operation Mode': 0,
        'Sample count': '1000448',
        'Sample rate': '1000000',
        'Stop Options': 1,
        'Threshold Level': '1',
        'Using Clock Negedge': true,
        'Using External Clock': 0,
        Version: 3,
      }),
    )
    expect(Object.fromEntries(session?.options ?? [])).toEqual({
      operationMode: { type: 'choice', value: 'buffer' },
      bufferOption: { type: 'choice', value: 'upload' },
      channelMode: { type: 'choice', value: '0' },
      vth: { type: 'number', value: 1 },
      filter: { type: 'bool', value: true },
      rle: { type: 'bool', value: false },
      externalClock: { type: 'bool', value: false },
      clockFallingEdge: { type: 'bool', value: true },
      samplerate: { type: 'integer', value: 1_000_000 },
      sampleLimit: { type: 'integer', value: 1_000_448 },
    })
    expect(session?.captureMode).toBe('loop')
  })

  it('read the DSLogic options of older DSView sessions by name', () => {
    // DSView's DSLogic1.def.dsc, version 2.
    const session = parseSession(
      JSON.stringify({
        'Channel Mode': 'Use Channels 0~1 (Max 200MHz)',
        Device: 'DSLogic',
        'Enable RLE Compress': 1,
        'Filter Targets': 'None',
        'Operation Mode': 'Stream模式',
        'Stop Options': '立即停止',
        'Threshold Level': '5.0V Level',
        Version: 2,
      }),
    )
    expect(Object.fromEntries(session?.options ?? [])).toEqual({
      operationMode: { type: 'choice', value: 'stream' },
      bufferOption: { type: 'choice', value: 'stop' },
      threshold: { type: 'choice', value: 'v50' },
      filter: { type: 'bool', value: false },
      rle: { type: 'bool', value: true },
    })
    expect(session?.captureMode).toBeNull()
  })

  it('apply modes before the settings they decide', () => {
    expect(optionRank('operationMode')).toBeLessThan(optionRank('samplerate'))
    expect(optionRank('channelMode')).toBeLessThan(optionRank('sampleLimit'))
  })
})
