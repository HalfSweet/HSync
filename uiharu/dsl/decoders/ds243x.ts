// SPDX-License-Identifier: Apache-2.0
/** Original bounded consumer of the canonical native network contract. */
import type { HexU64, Provenance, Range, Tick, bool, u8, u16 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
import { onewireEvents, type OneWireEvent } from '../contracts/onewire'
function crc16(crc: u16, byte: u8): u16 {
  let value: u16 = crc ^ byte
  for (let bit: u8 = 0; bit < 8; bit += 1)
    value = (value & 1) !== 0 ? (value >> 1) ^ 0xa001 : value >> 1
  return value
}
export type Model = 'ds2432' | 'ds2433'
export interface Config {
  model: Model
}
export type Command =
  | 'write-scratchpad'
  | 'read-scratchpad'
  | 'copy-scratchpad'
  | 'read-memory'
  | 'load-secret'
  | 'compute-secret'
  | 'authenticated-page'
  | 'unknown'
export type Field = 'scratchpad' | 'memory' | 'authorization' | 'mac' | 'es' | 'padding' | 'raw'
export type Reason = 'truncated' | 'input-error' | 'family' | 'padding'
export type Event =
  | { kind: 'rom'; id: HexU64; crcOk: bool; model: Model }
  | { kind: 'command'; code: u8; command: Command }
  | { kind: 'address'; address: u16 }
  | { kind: 'data'; field: Field; index: u16; value: u8 }
  | { kind: 'crc16'; value: u16; ok: bool }
  | { kind: 'status'; value: u8; success: bool }
  | { kind: 'error'; reason: Reason }
export const events = eventSchema<Event>('ds243x.events', 1).roles({
  rom: { role: 'address', errorWhen: { crcOk: false } },
  command: 'command',
  address: 'address',
  data: 'data',
  crc16: { role: 'check', errorWhen: { ok: false } },
  status: 'control',
  error: 'error',
})
function command(b: u8, model: Model): Command {
  switch (b) {
    case 0x0f:
      return 'write-scratchpad'
    case 0xaa:
      return 'read-scratchpad'
    case 0x55:
      return 'copy-scratchpad'
    case 0xf0:
      return 'read-memory'
    case 0x5a:
      return model === 'ds2432' ? 'load-secret' : 'unknown'
    case 0x33:
      return model === 'ds2432' ? 'compute-secret' : 'unknown'
    case 0xa5:
      return model === 'ds2432' ? 'authenticated-page' : 'unknown'
    default:
      return 'unknown'
  }
}
class Ds243x extends StreamDecoder<Config, OneWireEvent, Event> {
  active = false
  authenticating = false
  enabled = false
  model: Model = 'ds2432'
  code: u8 | undefined = undefined
  phase: u8 = 0
  address: u16 = 0
  index: u16 = 0
  left: u16 = 0
  low: u8 = 0
  crc: u16 = 0
  first: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    const p = this.first
    if (p !== undefined) return p.span.start
    return undefined
  }
  clear(): void {
    this.active = false
    this.authenticating = false
    this.enabled = false
    this.model = this.config.model
    this.code = undefined
    this.phase = 0
    this.address = 0
    this.index = 0
    this.left = 0
    this.first = undefined
  }
  incomplete(): bool {
    return (
      this.first !== undefined ||
      (this.code !== undefined && this.phase < 2) ||
      (this.phase === 2 && (this.code === 0xaa || this.code === 0x55 || this.code === 0x5a)) ||
      this.phase === 4 ||
      this.phase === 6 ||
      this.phase === 7 ||
      (this.code === 0x55 && this.model === 'ds2432' && this.phase === 3 && this.left !== 0)
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
  async byte(b: u8, at: Provenance): Promise<void> {
    const code = this.code
    if (code === undefined) {
      this.code = b
      this.crc = crc16(0, b)
      this.index = 0
      this.phase = 0
      const cmd = command(b, this.model)
      await this.out.emit({ kind: 'command', code: b, command: cmd }, at)
      if (cmd === 'unknown') this.phase = 9
      return
    }
    if (this.phase === 9) {
      await this.out.emit({ kind: 'data', field: 'raw', index: this.index, value: b }, at)
      this.index = (this.index + 1) & 0xffff
      return
    }
    if (this.phase === 0) {
      this.address = b
      this.crc = crc16(this.crc, b)
      this.first = at
      this.phase = 1
      return
    }
    if (this.phase === 1) {
      this.address |= (b as u16) << 8
      this.crc = crc16(this.crc, b)
      const p = this.first
      this.first = undefined
      await this.out.emit(
        { kind: 'address', address: this.address },
        p === undefined ? at : over(p.span.start, at.span.end, at.settled),
      )
      this.phase = 2
      this.left = this.model === 'ds2432' ? 8 : 32 - (this.address & 31)
      if (code === 0xa5) this.left = 32 - (this.address & 31)
      return
    }
    if (this.phase === 2 && (code === 0xaa || code === 0x55 || code === 0x5a)) {
      await this.out.emit({ kind: 'data', field: 'es', index: 0, value: b }, at)
      this.crc = crc16(this.crc, b)
      this.phase = 3
      if (code === 0x55 && this.model === 'ds2432') this.left = 20
      return
    }
    if (this.phase === 4) {
      this.low = b
      this.first = at
      this.phase = 5
      return
    }
    if (this.phase === 5) {
      const p = this.first
      const value: u16 = this.low | ((b as u16) << 8)
      await this.out.emit(
        { kind: 'crc16', value, ok: value === (this.crc ^ 0xffff) },
        p === undefined ? at : over(p.span.start, at.span.end, at.settled),
      )
      this.first = undefined
      if (code === 0xa5 && !this.authenticating) {
        this.left = 20
        this.phase = 6
        this.crc = 0
        this.index = 0
        this.authenticating = true
      } else this.phase = 8
      return
    }
    if (this.phase === 6) {
      await this.out.emit({ kind: 'data', field: 'mac', index: this.index, value: b }, at)
      this.crc = crc16(this.crc, b)
      this.index += 1
      this.left -= 1
      if (this.left === 0) this.phase = 4
      return
    }
    if (this.phase === 7) {
      await this.out.emit({ kind: 'data', field: 'padding', index: 0, value: b }, at)
      if (b !== 0xff) await this.out.emit({ kind: 'error', reason: 'padding' }, at)
      this.crc = crc16(this.crc, b)
      this.phase = 4
      return
    }
    if (
      code === 0x33 ||
      code === 0x5a ||
      (code === 0x55 && (this.model === 'ds2433' || this.left === 0)) ||
      this.phase === 8
    ) {
      await this.out.emit({ kind: 'status', value: b, success: b === 0xaa || b === 0x55 }, at)
      this.phase = 8
      return
    }
    if (code === 0x55 && this.model === 'ds2432') {
      await this.out.emit({ kind: 'data', field: 'mac', index: this.index, value: b }, at)
      this.index += 1
      this.left -= 1
      return
    }
    if (code === 0x0f || code === 0xaa || code === 0xa5) {
      await this.out.emit(
        {
          kind: 'data',
          field: code === 0xa5 ? 'memory' : 'scratchpad',
          index: this.index,
          value: b,
        },
        at,
      )
      this.crc = crc16(this.crc, b)
      this.index += 1
      this.left -= 1
      this.phase = 3
      if (this.left === 0) {
        if (code === 0xa5) this.phase = 7
        else if (code === 0x0f || this.model === 'ds2432') this.phase = 4
        else this.phase = 8
      }
      return
    }
    await this.out.emit({ kind: 'data', field: 'memory', index: this.index, value: b }, at)
    this.index = (this.index + 1) & 0xffff
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
          (e.code === 0xcc ||
            e.code === 0x3c ||
            e.code === 0x55 ||
            e.code === 0x69 ||
            e.code === 0x33)
        continue
      }
      if (e.kind === 'rom') {
        if (e.family === 0x33) this.model = 'ds2432'
        else if (e.family === 0x23) this.model = 'ds2433'
        else {
          this.enabled = false
          await this.out.emit({ kind: 'error', reason: 'family' }, at)
        }
        this.enabled = this.enabled && e.crcOk
        await this.out.emit({ kind: 'rom', id: e.id, crcOk: e.crcOk, model: this.model }, at)
        continue
      }
      if (e.kind === 'end') {
        await this.close(at)
        continue
      }
      if (!this.enabled) continue
      if (e.kind === 'byte') await this.byte(e.value, at)
      if (e.kind === 'bits') {
        await this.out.emit({ kind: 'error', reason: 'input-error' }, at)
        this.clear()
      }
    }
  }
}

export const device = defineStreamDecoder({
  name: 'ds243x',
  category: 'device',
  config: { defaults: { model: 'ds2432' } },
  input: onewireEvents,
  output: events,
  decoder: Ds243x,
  locales: {
    'en-US': {
      name: 'DS243X',
      summary: 'DS2432/DS2433 scratchpad, memory, CRC16 and authentication transport',
      fields: { model: 'Model' },
      values: { model: { ds2432: 'DS2432', ds2433: 'DS2433' } },
      events: {
        rom: 'Rom',
        command: 'Command',
        address: 'Address',
        data: 'Data',
        crc16: 'Crc16',
        status: 'Status',
        error: 'Error',
      },
      eventFields: {
        id: 'Id',
        crcOk: 'Crcok',
        model: 'Model',
        code: 'Code',
        command: 'Command',
        address: 'Address',
        field: 'Field',
        index: 'Index',
        value: 'Value',
        ok: 'Ok',
        success: 'Success',
        reason: 'Reason',
      },
    },
    'zh-CN': {
      name: 'DS243x',
      summary: 'DS2432/DS2433暂存器、存储器、CRC16和认证传输',
      fields: { model: '设备型号' },
      values: { model: { ds2432: 'DS2432', ds2433: 'DS2433' } },
      events: {
        rom: 'ROM编码',
        command: '功能命令',
        address: '目标地址',
        data: '数据',
        crc16: 'CRC16校验',
        status: '完成状态',
        error: '协议错误',
      },
      eventFields: {
        id: 'ROM编码',
        crcOk: 'CRC有效',
        model: '设备型号',
        code: '命令码',
        command: '功能命令',
        address: '目标地址',
        field: '数据字段',
        index: '字节索引',
        value: '数值',
        ok: '校验有效',
        success: '执行成功',
        reason: '错误原因',
      },
    },
  },
})
