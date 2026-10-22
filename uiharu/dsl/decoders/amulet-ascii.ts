// SPDX-License-Identifier: Apache-2.0
import type { Provenance, Range, Tick, bool, u8, u16, u32, u64 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over, join } from 'uiharu'
import { uartEvents, type UartEvent } from './uart'
export type EndReason = 'complete' | 'incomplete' | 'aborted' | 'invalid' | 'unsupported'
export interface DeviceConfig {
  maxBytes: u32
}
function validate(config: DeviceConfig): string | undefined {
  if (config.maxBytes < 16 || config.maxBytes > 1048576) return 'decode.config.length'
  return undefined
}
export type DeviceEvent =
  | { kind: 'begin'; frame: u64; code: u8 }
  | { kind: 'byte'; frame: u64; offset: u32; value: u8 }
  | { kind: 'field'; frame: u64; field: Field; value: u32; valid: bool }
  | { kind: 'end'; frame: u64; reason: EndReason }
export const deviceEvents = eventSchema<DeviceEvent>('amulet-ascii.events', 1).roles({
  begin: 'command',
  byte: 'data',
  field: { role: 'data', errorWhen: { valid: false } },
  end: 'framing',
})

/** Default Amulet ASCII opcodes, GEMstudio Pro User's Guide (2018), ASCII Commands.
 * Line/rectangle/filled-rectangle and echoed replies use 16 coordinate hex digits,
 * six color digits and one line-weight digit; customized opcode maps remain unsupported. */
export type Field =
  | 'address'
  | 'value'
  | 'element'
  | 'page'
  | 'checksum'
  | 'ack'
  | 'nack'
  | 'point-1-x'
  | 'point-1-y'
  | 'point-2-x'
  | 'point-2-y'
  | 'delta-x'
  | 'delta-y'
  | 'blue'
  | 'green'
  | 'red'
  | 'line-weight'
  | 'unused-line-weight'
function hex(byte: u8): u8 | undefined {
  if (byte >= 48 && byte <= 57) return byte - 48
  if (byte >= 65 && byte <= 70) return byte - 55
  if (byte >= 97 && byte <= 102) return byte - 87
  return undefined
}
function graphic(code: u8): bool {
  return (
    code === 0xd9 ||
    code === 0xda ||
    code === 0xdb ||
    code === 0xe9 ||
    code === 0xea ||
    code === 0xeb
  )
}
function digits(code: u8): u8 {
  if (graphic(code)) return 23
  if (
    code === 0xd0 ||
    code === 0xd1 ||
    code === 0xd2 ||
    code === 0xd3 ||
    code === 0xd4 ||
    code === 0xd8 ||
    code === 0xe8 ||
    code === 0xdd ||
    code === 0xde ||
    code === 0xf4
  )
    return 2
  if (code === 0xd5 || code === 0xe0 || code === 0xe5 || code === 0xef || code === 0xf3) return 4
  if (code === 0xd6 || code === 0xe1 || code === 0xe6) return 6
  if (code === 0xf5 || code === 0xf6 || code === 0xf7) return 10
  return 0
}
function terminated(code: u8): bool {
  return (
    code === 0xd7 ||
    code === 0xe2 ||
    code === 0xe3 ||
    code === 0xe4 ||
    code === 0xe7 ||
    code === 0xdf ||
    code === 0xed ||
    code === 0xee ||
    code === 0xf2
  )
}
function width(code: u8): u8 {
  if (code === 0xee || code === 0xf2) return 4
  return 2
}
class Device extends StreamDecoder<DeviceConfig, UartEvent, DeviceEvent> {
  frameAt: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    return this.frameAt?.span.start
  }
  open = false
  frame: u64 = 0
  code: u8 = 0
  position: u32 = 0
  value: u32 = 0
  accumulator: u32 = 0
  nibbleCount: u8 = 0
  sum: u16 = 0
  invalid = false

  override async onGap(range: Range): Promise<void> {
    await this.cut(range.start)
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
  }
  async cut(tick: Tick): Promise<void> {
    if (this.open) await this.finish('aborted', over(tick, tick, tick))
  }
  async finish(reason: EndReason, at: Provenance): Promise<void> {
    if (this.open) await this.out.emit({ kind: 'end', frame: this.frame, reason }, at)
    this.open = false
    this.frameAt = undefined
  }
  async field(field: Field, value: u32, valid: bool, at: Provenance): Promise<void> {
    const first = this.frameAt
    const placed = first === undefined ? at : join(first, at)
    await this.out.emit({ kind: 'field', frame: this.frame, field, value, valid }, placed)
  }

  override async run(): Promise<void> {
    for (;;) {
      const received = await this.input.next()
      const event = received.event
      const at = received.at
      if (event.kind === 'break') {
        await this.cut(at.span.start)
        continue
      }
      if (event.kind === 'frame') {
        if (event.value > 255 || !event.framingOk || event.parity === 'error') {
          if (this.open) await this.finish('invalid', at)
          continue
        }
        const byte = event.value as u8
        if (!this.open) {
          this.frame += 1
          this.open = true
          this.frameAt = at
          this.code = byte
          this.position = 0
          this.value = 0
          this.accumulator = 0
          this.nibbleCount = 0
          this.sum = byte
          this.invalid = false
          await this.out.emit({ kind: 'begin', frame: this.frame, code: byte }, at)
          if (byte === 0xf0 || byte === 0xf1) {
            await this.field(byte === 0xf0 ? 'ack' : 'nack', byte, true, at)
            await this.finish('complete', at)
          } else if (byte !== 0xa0 && digits(byte) === 0 && !terminated(byte))
            await this.finish('unsupported', at)
          continue
        }
        await this.out.emit(
          { kind: 'byte', frame: this.frame, offset: this.position, value: byte },
          at,
        )
        this.position += 1
        if (this.position > this.config.maxBytes) {
          await this.finish('invalid', at)
          continue
        }
        if (this.code === 0xa0) {
          this.sum = (this.sum + byte) & 255
          if (this.position === 1 && byte !== 2) this.invalid = true
          if (this.position === 2) this.value = byte * 256
          if (this.position === 3) this.value += byte
          if (this.position === 4) {
            await this.field('page', this.value, !this.invalid, at)
            await this.field('checksum', this.sum, this.sum === 0, at)
            await this.finish(this.invalid || this.sum !== 0 ? 'invalid' : 'complete', at)
          }
          continue
        }
        if (graphic(this.code)) {
          const digit = hex(byte)
          if (digit === undefined) this.invalid = true
          else this.accumulator = (this.accumulator << 4) | digit
          let field: Field = 'point-1-x'
          let complete = false
          if (this.position === 4) {
            field = 'point-1-x'
            complete = true
          } else if (this.position === 8) {
            field = 'point-1-y'
            complete = true
          } else if (this.position === 12) {
            field = this.code === 0xd9 || this.code === 0xe9 ? 'point-2-x' : 'delta-x'
            complete = true
          } else if (this.position === 16) {
            field = this.code === 0xd9 || this.code === 0xe9 ? 'point-2-y' : 'delta-y'
            complete = true
          } else if (this.position === 18) {
            field = 'blue'
            complete = true
          } else if (this.position === 20) {
            field = 'green'
            complete = true
          } else if (this.position === 22) {
            field = 'red'
            complete = true
          } else if (this.position === 23) {
            field = this.code === 0xdb || this.code === 0xeb ? 'unused-line-weight' : 'line-weight'
            complete = true
          }
          if (complete) {
            await this.field(field, this.accumulator, !this.invalid, at)
            this.accumulator = 0
          }
          if (this.position === 23) await this.finish(this.invalid ? 'invalid' : 'complete', at)
          continue
        }
        if (terminated(this.code) && this.position > 2 && byte === 0) {
          await this.finish(this.invalid || this.nibbleCount !== 0 ? 'invalid' : 'complete', at)
          continue
        }
        if (
          terminated(this.code) &&
          this.position > 2 &&
          (this.code === 0xd7 || this.code === 0xe2 || this.code === 0xe3 || this.code === 0xe7)
        )
          continue
        const digit = hex(byte)
        if (digit === undefined) {
          this.invalid = true
        } else {
          this.accumulator = (this.accumulator << 4) | digit
          this.nibbleCount += 1
          if (this.position === 2) {
            await this.field('address', this.accumulator, true, at)
            this.accumulator = 0
            this.nibbleCount = 0
          } else if (
            this.position > 2 &&
            terminated(this.code) &&
            this.nibbleCount === width(this.code)
          ) {
            await this.field('element', this.accumulator, true, at)
            this.accumulator = 0
            this.nibbleCount = 0
          }
        }
        if (!terminated(this.code) && this.position === digits(this.code)) {
          if (this.position > 2) await this.field('value', this.accumulator, !this.invalid, at)
          await this.finish(this.invalid ? 'invalid' : 'complete', at)
        }
      }
    }
  }
}

export const device = defineStreamDecoder({
  name: 'amulet-ascii',
  category: 'device',
  input: uartEvents,
  output: deviceEvents,
  config: { defaults: { maxBytes: 4096 }, validate },
  locales: {
    'en-US': {
      name: 'Amulet ASCII',
      summary:
        'Default ASCII scalar/string/array commands, graphics coordinates/color and page checksum',
      fields: { maxBytes: 'Maximum message bytes' },
      events: { begin: 'Begin', byte: 'Byte', field: 'Field', end: 'End' },
      eventFields: {
        frame: 'Frame',
        code: 'Command',
        offset: 'Offset',
        value: 'Value',
        field: 'Field',
        valid: 'Valid',
        reason: 'End reason',
      },
    },
    'zh-CN': {
      name: 'Amulet ASCII',
      summary: '默认ASCII标量/字符串/数组、图形坐标/颜色及页面校验',
      fields: { maxBytes: '消息字节上限' },
      events: { begin: '开始', byte: '字节', field: '字段', end: '结束' },
      eventFields: {
        frame: '帧',
        code: '命令',
        offset: '偏移',
        value: '值',
        field: '字段',
        valid: '有效',
        reason: '结束原因',
      },
    },
  },
  decoder: Device,
})
