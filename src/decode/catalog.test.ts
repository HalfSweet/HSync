import { describe, expect, it } from 'vitest'

import {
  baseTypes,
  buildEntry,
  DEFAULT_CATALOG,
  entryChannels,
  entryKinds,
  entryLabel,
  isEntry,
  nodeConfig,
  stackableOn,
  typeOf,
} from './catalog'

const uart = typeOf('uart')
const i2c = typeOf('i2c')
const eeprom = typeOf('eeprom24xx')

describe('the built-in catalog', () => {
  it('lists unique node types with their ports', () => {
    expect(new Set(DEFAULT_CATALOG.map((type) => type.name)).size).toBe(DEFAULT_CATALOG.length)
    expect(uart?.inputs[0]?.schema).toBe('logic.samples')
    expect(uart?.outputs[0]).toEqual({ port: 'events', schema: 'uart.events' })
    expect(baseTypes(DEFAULT_CATALOG).map((type) => type.name)).toContain('uart')
    expect(baseTypes(DEFAULT_CATALOG).map((type) => type.name)).not.toContain('eeprom24xx')
  })

  it('offers the new transports and the Arm JTAG debug layer', () => {
    const bases = baseTypes(DEFAULT_CATALOG).map((type) => type.name)
    expect(bases).toEqual(expect.arrayContaining(['qspi', 'sdcard-sd', 'swim']))
    expect(bases).not.toContain('jtag-dp')
    const jtag = typeOf('jtag')!
    const debug = typeOf('jtag-dp')!
    expect(stackableOn(jtag, DEFAULT_CATALOG).map((type) => type.name)).toContain('jtag-dp')
    const entry = buildEntry(jtag, {}, [{ type: debug, config: {} }])
    expect(entry.connections?.[1]).toEqual({ from: 'jtag.events', to: 'jtag-dp.bus' })
    expect(entry.expose?.['jtag-dp']).toBe('jtag-dp.events')
  })

  it('knows which types stack on which', () => {
    expect(stackableOn(i2c!, DEFAULT_CATALOG).map((type) => type.name)).toContain('eeprom24xx')
    expect(stackableOn(typeOf('spi')!, DEFAULT_CATALOG).map((type) => type.name)).toContain(
      'spi-nor',
    )
    expect(stackableOn(uart!, DEFAULT_CATALOG).map((type) => type.name)).toContain('modbus-rtu')
    expect(stackableOn(eeprom!, DEFAULT_CATALOG)).toEqual([])
  })

  it('offers device profiles with localized options and a three-layer Modbus path', () => {
    expect(stackableOn(i2c!, DEFAULT_CATALOG).map((type) => type.name)).toEqual(
      expect.arrayContaining(['ds1307', 'lm75', 'mcp230xx', 'edid', 'ltc26x7', 'atsha204a']),
    )
    const sensor = typeOf('lm75')!
    expect(sensor.locales?.['en-US']?.fields.resolution).toBeTruthy()
    expect(sensor.locales?.['zh-CN']?.fields.resolution).toBeTruthy()
    const rtu = typeOf('modbus-rtu')!
    const pdu = typeOf('modbus-pdu')!
    expect(stackableOn(rtu, DEFAULT_CATALOG).map((type) => type.name)).toContain('modbus-pdu')
    const entry = buildEntry(uart!, {}, [
      { type: rtu, config: {} },
      { type: pdu, config: { direction: 'response' } },
    ])
    expect(entry.connections).toEqual([
      { from: 'source', to: 'uart.logic' },
      { from: 'uart.events', to: 'modbus-rtu.uart' },
      { from: 'modbus-rtu.events', to: 'modbus-pdu.bus' },
    ])
    expect(entry.nodes['modbus-pdu']?.config).toEqual({ direction: 'response' })
  })

  it.each([
    ['cjtag', 'jtag-chain', 'jtag-dp', 'arm-memap'],
    ['swd', 'swd-dp'],
    ['spi', 'avr-isp', 'avr-isp-device'],
    ['onewire-link', 'onewire', 'ds2408'],
    ['onewire-link', 'onewire', 'ds243x'],
    ['onewire-link', 'onewire', 'ds28ea00'],
    ['mdio', 'cfp'],
    ['am230x', 'am230x-values'],
    ['uart', 'midi', 'midi-semantics'],
    ['uart', 'dmx512-analysis'],
    ['uart', 'lin-profile'],
  ])('offers the completed %s chain with compatible ports', (...names) => {
    const types = names.map((name) => typeOf(name)!)
    for (const type of types) expect(type, names.join(' → ')).toBeDefined()
    const entry = buildEntry(
      types[0]!,
      {},
      types.slice(1).map((type) => ({ type, config: {} })),
    )
    expect(entryKinds(entry)).toEqual(names)
    expect(entry.connections).toHaveLength(names.length)
    const leaf = types.at(-1)!
    expect(leaf.locales?.['en-US']?.name).toBeTruthy()
    expect(leaf.locales?.['zh-CN']?.name).toBeTruthy()
  })
})

describe('decoder entries', () => {
  it('build a bus with its configuration and label', () => {
    const entry = buildEntry(uart!, { line: 5, baud: 115200, inverted: undefined }, [], ' Console ')
    expect(entry).toEqual({
      nodes: { uart: { type: 'uart', config: { line: 5, baud: 115200 } } },
      connections: [{ from: 'source', to: 'uart.logic' }],
      expose: { uart: 'uart.events' },
      label: 'Console',
    })
    expect(isEntry(entry)).toBe(true)
    expect(isEntry({ id: '1:uart' })).toBe(false)
    expect(entryKinds(entry)).toEqual(['uart'])
    expect(entryLabel(entry, (type) => type.toUpperCase())).toBe('Console')
    expect(entryLabel({ nodes: entry.nodes }, (type) => type.toUpperCase())).toBe('UART')
  })

  it('stack a device decoder on the bus', () => {
    const entry = buildEntry(
      i2c!,
      { scl: 1, sda: 0 },
      [{ type: eeprom!, config: { address_bytes: 2, device_address: null } }],
      undefined,
      { numbers: 'dec' },
    )
    expect(Object.keys(entry.nodes)).toEqual(['i2c', 'eeprom24xx'])
    expect(entry.connections).toEqual([
      { from: 'source', to: 'i2c.logic' },
      { from: 'i2c.events', to: 'eeprom24xx.bus' },
    ])
    expect(entry.expose).toEqual({ i2c: 'i2c.events', eeprom24xx: 'eeprom24xx.operations' })
    expect(entry.nodes.eeprom24xx?.config).toEqual({ address_bytes: 2, device_address: null })
    expect(entry.label).toBeUndefined()
    expect(entry.show).toEqual({ numbers: 'dec' })
  })

  it('tell which channels an entry reads, defaults included', () => {
    const entry = buildEntry(i2c!, { scl: 9, sda: 2 })
    expect(entryChannels(entry)).toEqual([
      { node: 'i2c', field: 'scl', role: 'scl', index: 9 },
      { node: 'i2c', field: 'sda', role: 'sda', index: 2 },
    ])
    expect(entryChannels({ nodes: { u: { type: 'uart' } } })).toEqual([
      { node: 'u', field: 'line', role: 'line', index: 0 },
    ])
    expect(entryChannels({ nodes: { p: { type: 'parallel', config: { data: [3, 4] } } } })).toEqual(
      [
        { node: 'p', field: 'data', role: 'data0', index: 3 },
        { node: 'p', field: 'data', role: 'data1', index: 4 },
      ],
    )
    expect(nodeConfig({ type: 'uart', config: { line: 5 } })).toMatchObject({
      line: 5,
      baud: 115200,
      data_bits: 8,
      parity: 'none',
    })
    expect(entryChannels({ nodes: { x: { type: 'no-such-type' } } })).toEqual([])
  })
})
