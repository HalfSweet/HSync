// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type -- Empty DSL configuration records carry IR meaning. */
/** Original bounded consumer of the canonical native network contract. */
import type { HexU64, Provenance, Range, Tick, bool, u8, u16, u64 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
import { onewireEvents, type OneWireEvent } from '../contracts/onewire'
function crc16(crc: u16, byte: u8): u16 {
  let value: u16 = crc ^ byte
  for (let bit: u8 = 0; bit < 8; bit += 1)
    value = (value & 1) !== 0 ? (value >> 1) ^ 0xa001 : value >> 1
  return value
}
export interface Config {}
export type Command =
  | 'read-registers'
  | 'channel-read'
  | 'channel-write'
  | 'conditional-search'
  | 'reset-latches'
  | 'disable-test'
  | 'unknown'
export type Field = 'register' | 'pio-sample' | 'pio-output' | 'confirmation' | 'raw'
export type Check = 'inverse' | 'crc16' | 'confirmation'
export type Reason = 'truncated' | 'input-error' | 'address' | 'family'
export type Event =
  | { kind: 'rom'; id: HexU64; crcOk: bool }
  | { kind: 'command'; code: u8; command: Command }
  | { kind: 'address'; address: u16 }
  | { kind: 'data'; field: Field; address: u16 | undefined; value: u8 }
  | { kind: 'check'; check: Check; ok: bool; value: u16 }
  | { kind: 'error'; reason: Reason }
export const events = eventSchema<Event>('ds2408.events', 1).roles({
  rom: { role: 'address', errorWhen: { crcOk: false } },
  command: 'command',
  address: 'address',
  data: 'data',
  check: { role: 'check', errorWhen: { ok: false } },
  error: 'error',
})
function command(code: u8): Command {
  switch (code) {
    case 0xf0:
      return 'read-registers'
    case 0xf5:
      return 'channel-read'
    case 0x5a:
      return 'channel-write'
    case 0xcc:
      return 'conditional-search'
    case 0xc3:
      return 'reset-latches'
    case 0x3c:
      return 'disable-test'
    default:
      return 'unknown'
  }
}
function crc8(crc: u8, byte: u8): u8 {
  let value: u8 = crc ^ byte
  for (let bit: u8 = 0; bit < 8; bit += 1)
    value = (value & 1) !== 0 ? (value >> 1) ^ 0x8c : value >> 1
  return value
}
class Ds2408 extends StreamDecoder<Config, OneWireEvent, Event> {
  active = false
  enabled = false
  code: u8 | undefined = undefined
  phase: u8 = 0
  address: u16 = 0
  crc: u16 = 0
  low: u8 = 0
  count: u8 = 0
  pending: Provenance | undefined = undefined
  special: u8 = 0
  rom: u64 = 0
  romCrc: u8 = 0
  override hold(): Tick | undefined {
    const p = this.pending
    if (p !== undefined) return p.span.start
    return undefined
  }
  clear(): void {
    this.active = false
    this.enabled = false
    this.code = undefined
    this.phase = 0
    this.pending = undefined
    this.special = 0
    this.rom = 0
    this.romCrc = 0
  }
  incomplete(): bool {
    return (
      this.pending !== undefined ||
      (this.code === 0x5a && this.phase !== 0) ||
      (this.code === 0xf0 && this.phase < 2) ||
      (this.code === 0xcc && this.phase < 2) ||
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
  async byte(b: u8, at: Provenance): Promise<void> {
    if (this.special !== 0) {
      if (this.special <= 8) {
        if (this.pending === undefined) this.pending = at
        this.rom |= (b as u64) << (((this.special - 1) * 8) as u64)
        this.romCrc = crc8(this.romCrc, b)
        this.special += 1
        if (this.special === 9) {
          const p = this.pending
          await this.out.emit(
            { kind: 'rom', id: this.rom, crcOk: this.romCrc === 0 },
            p === undefined ? at : over(p.span.start, at.span.end, at.settled),
          )
          this.pending = undefined
        }
        return
      }
      await this.out.emit(
        { kind: 'command', code: b, command: b === 0x3c ? 'disable-test' : 'unknown' },
        at,
      )
      this.special = 0
      this.enabled = false
      return
    }
    const code = this.code
    if (code === undefined) {
      this.code = b
      this.crc = crc16(0, b)
      this.phase = 0
      this.count = 0
      await this.out.emit({ kind: 'command', code: b, command: command(b) }, at)
      return
    }
    if (code === 0xf0 || code === 0xcc) {
      if (this.phase === 0) {
        this.address = b
        this.crc = crc16(this.crc, b)
        this.phase = 1
        this.pending = at
        return
      }
      if (this.phase === 1) {
        this.address |= (b as u16) << 8
        this.crc = crc16(this.crc, b)
        this.phase = 2
        const p = this.pending
        this.pending = undefined
        await this.out.emit(
          { kind: 'address', address: this.address },
          p === undefined ? at : over(p.span.start, at.span.end, at.settled),
        )
        if (
          (code === 0xcc && (this.address < 0x8b || this.address > 0x8d)) ||
          (code === 0xf0 && this.address > 0x8f)
        )
          await this.out.emit({ kind: 'error', reason: 'address' }, at)
        return
      }
      if (code === 0xf0 && this.address === 0x90 && this.phase < 4) {
        if (this.phase === 2) {
          this.low = b
          this.phase = 3
          this.pending = at
          return
        }
        const p = this.pending
        const value: u16 = this.low | ((b as u16) << 8)
        await this.out.emit(
          { kind: 'check', check: 'crc16', value, ok: value === (this.crc ^ 0xffff) },
          p === undefined ? at : over(p.span.start, at.span.end, at.settled),
        )
        this.pending = undefined
        this.phase = 4
        return
      }
      const address = this.address
      await this.out.emit({ kind: 'data', field: 'register', address, value: b }, at)
      if (code === 0xf0 && this.phase === 2 && address < 0x90) {
        this.crc = crc16(this.crc, b)
        this.address += 1
      } else if (code === 0xcc && address >= 0x8b && address <= 0x8d) this.address += 1
      return
    }
    if (code === 0xf5) {
      if (this.count < 32) {
        await this.out.emit({ kind: 'data', field: 'pio-sample', address: undefined, value: b }, at)
        this.crc = crc16(this.crc, b)
        this.count += 1
        return
      }
      if (this.count === 32) {
        this.low = b
        this.pending = at
        this.count = 33
        return
      }
      const p = this.pending
      const value: u16 = this.low | ((b as u16) << 8)
      await this.out.emit(
        { kind: 'check', check: 'crc16', value, ok: value === (this.crc ^ 0xffff) },
        p === undefined ? at : over(p.span.start, at.span.end, at.settled),
      )
      this.pending = undefined
      this.count = 0
      this.crc = 0
      return
    }
    if (code === 0x5a) {
      if (this.phase === 0) {
        this.low = b
        this.pending = at
        this.phase = 1
        await this.out.emit({ kind: 'data', field: 'pio-output', address: undefined, value: b }, at)
        return
      }
      if (this.phase === 1) {
        const p = this.pending
        await this.out.emit(
          { kind: 'check', check: 'inverse', value: b, ok: b === (this.low ^ 0xff) },
          p === undefined ? at : over(p.span.start, at.span.end, at.settled),
        )
        this.pending = undefined
        this.phase = 2
        return
      }
      if (this.phase === 2) {
        await this.out.emit({ kind: 'check', check: 'confirmation', value: b, ok: b === 0xaa }, at)
        this.phase = 3
        return
      }
      await this.out.emit({ kind: 'data', field: 'pio-sample', address: undefined, value: b }, at)
      this.phase = 0
      return
    }
    if (code === 0xc3) {
      await this.out.emit({ kind: 'check', check: 'confirmation', value: b, ok: b === 0xaa }, at)
      return
    }
    await this.out.emit({ kind: 'data', field: 'raw', address: undefined, value: b }, at)
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
            e.code === 0x33 ||
            e.code === 0x96)
        if (e.code === 0x96) this.special = 1
        continue
      }
      if (e.kind === 'rom') {
        this.enabled = this.enabled && e.crcOk && e.family === 0x29
        await this.out.emit({ kind: 'rom', id: e.id, crcOk: e.crcOk }, at)
        if (e.family !== 0x29) await this.out.emit({ kind: 'error', reason: 'family' }, at)
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
  name: 'ds2408',
  category: 'device',
  config: { defaults: {} },
  input: onewireEvents,
  output: events,
  decoder: Ds2408,
  locales: {
    'en-US': {
      name: 'DS2408',
      summary: 'PIO registers, channel samples, complemented writes and CRC16',
      fields: {},
      events: {
        rom: 'Rom',
        command: 'Command',
        address: 'Address',
        data: 'Data',
        check: 'Check',
        error: 'Error',
      },
      eventFields: {
        id: 'Id',
        crcOk: 'Crcok',
        code: 'Code',
        command: 'Command',
        address: 'Address',
        field: 'Field',
        value: 'Value',
        check: 'Check',
        ok: 'Ok',
        reason: 'Reason',
      },
    },
    'zh-CN': {
      name: 'DS2408',
      summary: 'PIO寄存器、通道采样、互补写入和CRC16',
      fields: {},
      events: {
        rom: 'ROM编码',
        command: '功能命令',
        address: '目标地址',
        data: '数据',
        check: '完整性校验',
        error: '协议错误',
      },
      eventFields: {
        id: 'ROM编码',
        crcOk: 'CRC有效',
        code: '命令码',
        command: '功能命令',
        address: '目标地址',
        field: '数据字段',
        value: '数值',
        check: '完整性校验',
        ok: '校验有效',
        reason: '错误原因',
      },
    },
  },
})
