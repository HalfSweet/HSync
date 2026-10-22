// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type, @typescript-eslint/no-unnecessary-type-assertion -- Empty DSL records and explicit integer-width casts carry IR meaning. */
/** Original bounded consumer of the canonical native network contract. */
import type { HexU64, Provenance, Range, Tick, bool, u8, u16, u32, u64 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
import { onewireEvents, type OneWireEvent } from '../contracts/onewire'
function crc8(crc: u8, byte: u8): u8 {
  let value: u8 = crc ^ byte
  for (let bit: u8 = 0; bit < 8; bit += 1)
    value = (value & 1) !== 0 ? (value >> 1) ^ 0x8c : value >> 1
  return value
}
export interface Config {}
export type Command =
  | 'write-scratchpad'
  | 'read-scratchpad'
  | 'copy-scratchpad'
  | 'convert'
  | 'power'
  | 'recall'
  | 'pio-read'
  | 'pio-write'
  | 'chain'
  | 'unknown'
export type Chain = 'off' | 'on' | 'done' | 'invalid'
export type Check = 'crc8' | 'inverse' | 'confirmation'
export type Reason = 'truncated' | 'family' | 'input-error'
export type Event =
  | { kind: 'rom'; id: HexU64; crcOk: bool }
  | { kind: 'command'; code: u8; command: Command }
  | { kind: 'data'; index: u8; value: u8 }
  | {
      kind: 'temperature'
      raw: u16
      microCelsius: u32
      negative: bool
      resolution: u8
      crcOk: bool
    }
  | { kind: 'pio'; a: bool; b: bool; latchA: bool; latchB: bool; valid: bool }
  | { kind: 'chain'; state: Chain; inverseOk: bool }
  | { kind: 'status'; value: u8; count: u8 }
  | { kind: 'check'; check: Check; ok: bool; value: u8 }
  | { kind: 'error'; reason: Reason }
export const events = eventSchema<Event>('ds28ea00.events', 1).roles({
  rom: { role: 'address', errorWhen: { crcOk: false } },
  command: 'command',
  data: 'data',
  temperature: { role: 'data', errorWhen: { crcOk: false } },
  pio: { role: 'data', errorWhen: { valid: false } },
  chain: { role: 'control', errorWhen: { inverseOk: false } },
  status: 'control',
  check: { role: 'check', errorWhen: { ok: false } },
  error: 'error',
})
function command(b: u8): Command {
  switch (b) {
    case 0x4e:
      return 'write-scratchpad'
    case 0xbe:
      return 'read-scratchpad'
    case 0x48:
      return 'copy-scratchpad'
    case 0x44:
      return 'convert'
    case 0xb4:
      return 'power'
    case 0xb8:
      return 'recall'
    case 0xf5:
      return 'pio-read'
    case 0xa5:
      return 'pio-write'
    case 0x99:
      return 'chain'
    default:
      return 'unknown'
  }
}
class Ds28ea00 extends StreamDecoder<Config, OneWireEvent, Event> {
  active = false
  enabled = false
  code: u8 | undefined = undefined
  index: u8 = 0
  low: u8 = 0
  raw: u16 = 0
  crc: u8 = 0
  resolution: u8 = 12
  first: Provenance | undefined = undefined
  special: u8 = 0
  rom: u64 = 0
  override hold(): Tick | undefined {
    const p = this.first
    if (p !== undefined) return p.span.start
    return undefined
  }
  clear(): void {
    this.active = false
    this.enabled = false
    this.code = undefined
    this.index = 0
    this.first = undefined
    this.special = 0
    this.rom = 0
    this.crc = 0
  }
  incomplete(): bool {
    return (
      this.first !== undefined ||
      (this.code === 0x4e && this.index < 3) ||
      (this.code === 0xa5 && this.index !== 0) ||
      (this.code === 0x99 && this.index < 3) ||
      this.special !== 0
    )
  }
  async close(at: Provenance): Promise<void> {
    if (this.incomplete()) await this.out.emit({ kind: 'error', reason: 'truncated' }, at)
    this.clear()
  }
  override async onGap(range: Range): Promise<void> {
    await this.close(over(range.start, range.start, range.start))
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.close(over(tick, tick, tick))
  }
  async pio(b: u8, at: Provenance): Promise<void> {
    await this.out.emit(
      {
        kind: 'pio',
        a: (b & 1) !== 0,
        latchA: (b & 2) !== 0,
        b: (b & 4) !== 0,
        latchB: (b & 8) !== 0,
        valid: ((b >> 4) ^ (b & 15)) === 15,
      },
      at,
    )
  }
  async byte(b: u8, at: Provenance): Promise<void> {
    if (this.special !== 0) {
      if (this.first === undefined) this.first = at
      this.rom |= (b as u64) << (((this.special - 1) * 8) as u64)
      this.crc = crc8(this.crc, b)
      this.special += 1
      if (this.special === 9) {
        const p = this.first
        await this.out.emit(
          { kind: 'rom', id: this.rom, crcOk: this.crc === 0 },
          p === undefined ? at : over(p.span.start, at.span.end, at.settled),
        )
        this.enabled = this.crc === 0 && (this.rom & 0xff) === 0x42
        this.special = 0
        this.first = undefined
        this.crc = 0
      }
      return
    }
    const code = this.code
    if (code === undefined) {
      this.code = b
      this.index = 0
      this.crc = 0
      await this.out.emit({ kind: 'command', code: b, command: command(b) }, at)
      return
    }
    if (code === 0xbe && this.index < 9) {
      if (this.index === 0) {
        this.raw = b
        this.first = at
      }
      if (this.index === 1) this.raw |= (b as u16) << 8
      if (this.index === 4) this.resolution = 9 + ((b >> 5) & 3)
      if (this.index < 8) this.crc = crc8(this.crc, b)
      await this.out.emit({ kind: 'data', index: this.index, value: b }, at)
      if (this.index === 8) {
        const p = this.first
        let raw: u16 = this.raw
        if (this.resolution === 9) raw = (this.raw & 0xfff8) as u16
        else if (this.resolution === 10) raw = (this.raw & 0xfffc) as u16
        else if (this.resolution === 11) raw = (this.raw & 0xfffe) as u16
        await this.out.emit(
          {
            kind: 'temperature',
            raw,
            microCelsius: ((raw & 0x8000) !== 0 ? 65536 - (raw as u32) : raw) * 62500,
            negative: (raw & 0x8000) !== 0,
            resolution: this.resolution,
            crcOk: this.crc === b,
          },
          p === undefined ? at : over(p.span.start, at.span.end, at.settled),
        )
        this.first = undefined
      }
      this.index += 1
      return
    }
    if (code === 0xf5) {
      await this.pio(b, at)
      return
    }
    if (code === 0xa5) {
      if (this.index === 0) {
        this.low = b
        this.first = at
        this.index = 1
        await this.out.emit({ kind: 'data', index: 0, value: b }, at)
        return
      }
      if (this.index === 1) {
        const p = this.first
        await this.out.emit(
          { kind: 'check', check: 'inverse', value: b, ok: b === (this.low ^ 0xff) },
          p === undefined ? at : over(p.span.start, at.span.end, at.settled),
        )
        this.first = undefined
        this.index = 2
        return
      }
      if (this.index === 2) {
        await this.out.emit({ kind: 'check', check: 'confirmation', value: b, ok: b === 0xaa }, at)
        this.index = 3
        return
      }
      await this.pio(b, at)
      this.index = 0
      return
    }
    if (code === 0x99) {
      if (this.index === 0) {
        this.low = b
        this.first = at
        this.index = 1
        return
      }
      if (this.index === 1) {
        const p = this.first
        let state: Chain = 'invalid'
        if (this.low === 0x3c) state = 'off'
        else if (this.low === 0x5a) state = 'on'
        else if (this.low === 0x96) state = 'done'
        await this.out.emit(
          { kind: 'chain', state, inverseOk: b === (this.low ^ 0xff) },
          p === undefined ? at : over(p.span.start, at.span.end, at.settled),
        )
        this.first = undefined
        this.index = 2
        return
      }
      await this.out.emit({ kind: 'check', check: 'confirmation', value: b, ok: b === 0xaa }, at)
      this.index = 3
      return
    }
    if (code === 0x44 || code === 0xb4 || code === 0xb8 || code === 0x48) {
      await this.out.emit({ kind: 'status', value: b, count: 8 }, at)
      return
    }
    await this.out.emit({ kind: 'data', index: this.index, value: b }, at)
    if (this.index < 255) this.index += 1
  }
  override async run(): Promise<void> {
    for (;;) {
      const r = await this.input.next()
      const e = r.event
      const at = r.at
      if (e.kind === 'begin') {
        this.clear()
        this.active = true
        this.enabled = e.presence
        continue
      }
      if (!this.active) continue
      if (e.kind === 'rom-command') {
        this.enabled =
          this.enabled &&
          (e.code === 0x0f ||
            e.code === 0xcc ||
            e.code === 0x3c ||
            e.code === 0x55 ||
            e.code === 0x69 ||
            e.code === 0x33)
        if (e.code === 0x0f) this.special = 1
        continue
      }
      if (e.kind === 'rom') {
        this.enabled = this.enabled && e.crcOk && e.family === 0x42
        await this.out.emit({ kind: 'rom', id: e.id, crcOk: e.crcOk }, at)
        if (e.family !== 0x42) await this.out.emit({ kind: 'error', reason: 'family' }, at)
        continue
      }
      if (e.kind === 'end') {
        await this.close(at)
        continue
      }
      if (!this.enabled) continue
      if (e.kind === 'byte') await this.byte(e.value, at)
      if (e.kind === 'bits') {
        if (this.code === 0x44 || this.code === 0xb4 || this.code === 0xb8 || this.code === 0x48)
          await this.out.emit({ kind: 'status', value: e.value, count: e.count }, at)
        else {
          await this.out.emit({ kind: 'error', reason: 'input-error' }, at)
          this.clear()
        }
      }
    }
  }
}

export const device = defineStreamDecoder({
  name: 'ds28ea00',
  category: 'device',
  config: { defaults: {} },
  input: onewireEvents,
  output: events,
  decoder: Ds28ea00,
  locales: {
    'en-US': {
      name: 'DS28EA00',
      summary: 'Thermometer scratchpad, PIO and chain commands',
      fields: {},
      events: {
        rom: 'Rom',
        command: 'Command',
        data: 'Data',
        temperature: 'Temperature',
        pio: 'Pio',
        chain: 'Chain',
        status: 'Status',
        check: 'Check',
        error: 'Error',
      },
      eventFields: {
        id: 'Id',
        crcOk: 'Crcok',
        code: 'Code',
        command: 'Command',
        index: 'Index',
        value: 'Value',
        negative: 'Negative temperature',
        raw: 'Raw',
        microCelsius: 'Microcelsius',
        resolution: 'Resolution',
        a: 'A',
        b: 'B',
        latchA: 'Latcha',
        latchB: 'Latchb',
        valid: 'Valid',
        state: 'State',
        inverseOk: 'Inverseok',
        count: 'Count',
        check: 'Check',
        ok: 'Ok',
        reason: 'Reason',
      },
    },
    'zh-CN': {
      name: 'DS28EA00',
      summary: '温度暂存器、PIO和链控制命令',
      fields: {},
      events: {
        rom: 'ROM编码',
        command: '功能命令',
        data: '数据',
        temperature: '温度',
        pio: 'PIO状态',
        chain: '链控制',
        status: '完成状态',
        check: '完整性校验',
        error: '协议错误',
      },
      eventFields: {
        id: 'ROM编码',
        crcOk: 'CRC有效',
        code: '命令码',
        command: '功能命令',
        index: '字节索引',
        value: '数值',
        raw: '原始温度',
        microCelsius: '温度绝对值（微摄氏度）',
        resolution: '分辨率',
        a: 'PIOA电平',
        b: 'PIOB电平',
        latchA: 'PIOA锁存',
        latchB: 'PIOB锁存',
        valid: '状态有效',
        state: '链状态',
        inverseOk: '反码有效',
        count: '位数',
        check: '完整性校验',
        ok: '校验有效',
        reason: '错误原因',
        negative: '负温度',
      },
    },
  },
})
