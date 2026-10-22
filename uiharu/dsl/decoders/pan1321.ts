// SPDX-License-Identifier: Apache-2.0
import type { Provenance, Range, Tick, bool, u8, u32, u64 } from 'uiharu'
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
export const deviceEvents = eventSchema<DeviceEvent>('pan1321.events', 1).roles({
  begin: 'command',
  byte: 'data',
  field: { role: 'data', errorWhen: { valid: false } },
  end: 'framing',
})

/** PAN1321 SPP AT reference profile. Facts attributed to libsigrokdecode PAN1321
 * (Uwe Hermann, 2012–2013); original bounded streaming parser, no copied implementation. */
export type Field = 'command' | 'declared-length' | 'mode' | 'status' | 'error-code'
class Device extends StreamDecoder<DeviceConfig, UartEvent, DeviceEvent> {
  frameAt: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    return this.frameAt?.span.start
  }
  open = false
  frame: u64 = 0
  position: u32 = 0
  prefix: u32 = 0
  command: u32 = 0
  kind: u8 = 0
  value: u32 = 0
  decimal = true
  declared = false
  remaining: u32 = 0
  cr = false
  invalid = false
  dropping = false
  trailer: u8 = 0

  override async onGap(range: Range): Promise<void> {
    await this.cut(range.start)
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
  }
  async cut(tick: Tick): Promise<void> {
    if (this.open) await this.finish('aborted', over(tick, tick, tick))
    this.dropping = true
    this.cr = false
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
          this.dropping = true
          this.cr = false
          continue
        }
        const byte = event.value as u8
        if (this.dropping) {
          if (
            this.prefix === 0x4552523d &&
            this.position > 4 &&
            byte !== 13 &&
            byte !== 10 &&
            (byte < 48 || byte > 57)
          )
            this.invalid = true
          if (this.cr && byte === 10) {
            if ((this.kind === 1 || this.kind === 2) && this.position !== 11) this.invalid = true
            if (this.kind === 6 && this.position !== 9) this.invalid = true
            if (this.prefix === 0x4552523d && this.position < 7) this.invalid = true
            this.dropping = false
            this.cr = false
          } else this.cr = byte === 13
          continue
        }
        if (!this.open) {
          this.frame += 1
          this.open = true
          this.frameAt = at
          this.position = 0
          this.prefix = 0
          this.command = 0
          this.kind = 0
          this.value = 0
          this.decimal = true
          this.declared = false
          this.remaining = 0
          this.trailer = 0
          this.cr = false
          this.invalid = false
          await this.out.emit({ kind: 'begin', frame: this.frame, code: byte }, at)
        }
        await this.out.emit(
          { kind: 'byte', frame: this.frame, offset: this.position, value: byte },
          at,
        )
        this.position += 1
        if (this.position > this.config.maxBytes) {
          await this.finish('invalid', at)
          this.dropping = true
          this.cr = byte === 13
          continue
        }
        if (this.position <= 4) this.prefix = (this.prefix << 8) | byte
        if (this.position >= 5 && this.position <= 8) this.command = (this.command << 8) | byte
        if (this.position === 8 && this.prefix === 0x41542b4a) {
          if (this.command === 0x4141433d) this.kind = 1
          else if (this.command === 0x50524f3d) this.kind = 2
          else if (this.command === 0x5344413d) this.kind = 3
          else if (this.command === 0x5345433d) this.kind = 4
          else if (this.command === 0x534c4e3d) this.kind = 5
          else if (this.command >> 8 === 0x524553) this.kind = 6
          await this.field('command', this.kind, this.kind !== 0, at)
        }
        if (this.kind === 3 && !this.declared && this.position > 8) {
          if (byte === 44) {
            this.declared = true
            this.remaining = this.value
            await this.field('declared-length', this.value, this.decimal && this.value > 0, at)
            if (!this.decimal || this.value === 0 || this.value > 65535) this.invalid = true
          } else if (byte >= 48 && byte <= 57 && this.value <= 6553)
            this.value = this.value * 10 + byte - 48
          else {
            this.decimal = false
            this.invalid = true
          }
          this.cr = false
          continue
        }
        if (this.kind === 3 && this.declared && this.remaining > 0) {
          this.remaining -= 1
          this.cr = false
          continue
        }
        if (this.kind === 3 && this.declared && this.remaining === 0) {
          if (this.trailer === 0) {
            if (byte !== 13) this.invalid = true
            this.trailer = 1
          } else if (this.trailer === 1) {
            if (byte !== 10) this.invalid = true
            this.trailer = 2
          } else this.invalid = true
        }
        if (this.kind === 1 || this.kind === 2) {
          if (this.position === 9) {
            await this.field('mode', byte >= 48 ? byte - 48 : byte, byte === 48 || byte === 49, at)
            if (byte !== 48 && byte !== 49) this.invalid = true
          }
        }
        if (
          this.prefix === 0x4552523d &&
          this.position > 4 &&
          byte >= 48 &&
          byte <= 57 &&
          this.value <= 6553
        )
          this.value = this.value * 10 + byte - 48
        if (
          this.prefix === 0x4552523d &&
          this.position > 4 &&
          byte !== 13 &&
          byte !== 10 &&
          (byte < 48 || byte > 57)
        )
          this.invalid = true
        if (this.cr && byte === 10) {
          if ((this.kind === 1 || this.kind === 2) && this.position !== 11) this.invalid = true
          if (this.kind === 6 && this.position !== 9) this.invalid = true
          if (this.prefix === 0x4552523d && this.position < 7) this.invalid = true
          if (this.prefix === 0x4552523d) await this.field('error-code', this.value, true, at)
          if (this.position === 4 && this.prefix === 0x4f4b0d0a)
            await this.field('status', 0, true, at)
          if (this.position === 5 && this.prefix >> 8 === 0x524f4b)
            await this.field('status', 1, true, at)
          await this.finish(
            this.invalid || this.remaining !== 0
              ? 'invalid'
              : this.kind === 0 &&
                  !(this.position === 4 && this.prefix === 0x4f4b0d0a) &&
                  !(this.position === 5 && this.prefix >> 8 === 0x524f4b) &&
                  this.prefix !== 0x4552523d
                ? 'unsupported'
                : 'complete',
            at,
          )
          continue
        }
        this.cr = byte === 13
      }
    }
  }
}

export const device = defineStreamDecoder({
  name: 'pan1321',
  category: 'device',
  input: uartEvents,
  output: deviceEvents,
  config: { defaults: { maxBytes: 4096 }, validate },
  locales: {
    'en-US': {
      name: 'PAN1321 SPP AT',
      summary: 'SPP AT command identifiers, modes, status and counted JSDA payloads',
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
      name: 'PAN1321 SPP AT',
      summary: 'SPP AT 命令、模式、状态及定长 JSDA 数据',
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
