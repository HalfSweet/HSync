// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Integer assertions select IR widths and truncation; TypeScript aliases alone do not encode these semantics. */

/**
 * ITM/DWT packets from unformatted asynchronous SWO UART bytes. Arm DDI0553B.y,
 * Part F1: https://community.arm.com/cfs-file/__key/communityserver-discussions-components-files/471/DDI0553B_5F00_y_5F00_armv8m_5F00_arm.pdf
 * Source packets, local timestamps, extensions, synchronization and overflow are decoded.
 * Global timestamps expose observed fragments; reserved headers stop interpretation until synchronization.
 */
import type { Provenance, Range, Tick, bool, u16, u32, u64, u8 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
import { traceByteEvents, type TraceByteEvent } from './arm-trace-transport'
export interface ArmItmConfig {
  startAligned: bool
}
export type ErrorReason =
  'aborted' | 'input-error' | 'unsupported-header' | 'overlong' | 'bad-sync' | 'malformed'
export type ArmItmEvent =
  | { kind: 'sync' }
  | { kind: 'overflow' }
  | { kind: 'software'; port: u8; page: u32 | undefined; bytes: u8; value: u32 }
  | { kind: 'hardware'; source: u8; bytes: u8; value: u32 }
  | { kind: 'event-counter'; flags: u8 }
  | { kind: 'exception'; exception: u16; action: u8 }
  | { kind: 'pc'; value: u32 }
  | { kind: 'sleep' }
  | { kind: 'timestamp'; delta: u32; control: u8 }
  | { kind: 'global-low'; fragment: u32; significantBits: u8; wrap: bool; clockChange: bool }
  | { kind: 'global-high'; fragment: u64; timestampBits: u8 }
  | { kind: 'extension'; hardware: bool; value: u32 }
  | { kind: 'error'; header: u8 | undefined; reason: ErrorReason }
export const armItmEvents = eventSchema<ArmItmEvent>('arm-itm.events', 1).roles({
  sync: 'framing',
  overflow: 'line',
  software: 'data',
  hardware: 'data',
  'event-counter': 'control',
  exception: 'control',
  pc: 'address',
  sleep: 'line',
  timestamp: 'control',
  'global-low': 'control',
  'global-high': 'control',
  extension: 'control',
  error: 'error',
})
class ArmItm extends StreamDecoder<ArmItmConfig, TraceByteEvent, ArmItmEvent> {
  aligned = this.config.startAligned
  page: u32 | undefined = undefined
  mode: u8 = 0
  header: u8 = 0
  size: u8 = 0
  remaining: u8 = 0
  count: u8 = 0
  wide: u64 = 0
  value: u32 = 0
  shift: u8 = 0
  start: Provenance | undefined = undefined
  zeros: u8 = 0
  zeroAt: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    const first = this.start
    if (first !== undefined) return first.span.start
    const zero = this.zeroAt
    if (zero !== undefined) return zero.span.start
    return undefined
  }
  clear(): void {
    this.mode = 0
    this.start = undefined
    this.zeros = 0
    this.zeroAt = undefined
  }
  async fail(reason: ErrorReason, at: Provenance): Promise<void> {
    const first = this.start
    const placed = first === undefined ? at : over(first.span.start, at.span.end, at.settled)
    await this.out.emit(
      { kind: 'error', header: this.mode === 0 ? undefined : this.header, reason },
      placed,
    )
    this.clear()
    this.aligned = false
    this.page = undefined
  }
  override async onGap(range: Range): Promise<void> {
    if (this.mode !== 0) await this.fail('aborted', over(range.start, range.start, range.start))
    this.clear()
    this.aligned = false
    this.page = undefined
  }
  override async onEnd(tick: Tick): Promise<void> {
    if (this.mode !== 0) await this.fail('aborted', over(tick, tick, tick))
    this.clear()
  }
  async finishSource(at: Provenance): Promise<void> {
    const first = this.start
    if (first === undefined) return
    const placed = over(first.span.start, at.span.end, at.settled)
    const source: u8 = this.header >> 3
    if ((this.header & 4) === 0)
      await this.out.emit(
        { kind: 'software', port: source, page: this.page, bytes: this.size, value: this.value },
        placed,
      )
    else {
      await this.out.emit({ kind: 'hardware', source, bytes: this.size, value: this.value }, placed)
      if (source === 0 && this.size === 1)
        await this.out.emit({ kind: 'event-counter', flags: this.value as u8 }, placed)
      if (source === 1 && this.size === 2)
        await this.out.emit(
          {
            kind: 'exception',
            exception: (this.value & 0x1ff) as u16,
            action: ((this.value >> 12) & 3) as u8,
          },
          placed,
        )
      if (source === 2 && this.size === 4)
        await this.out.emit({ kind: 'pc', value: this.value }, placed)
      if (source === 2 && this.size === 1 && this.value === 0)
        await this.out.emit({ kind: 'sleep' }, placed)
    }
    this.mode = 0
    this.start = undefined
  }
  async consume(byte: u8, at: Provenance): Promise<void> {
    if (this.mode === 5 || this.mode === 6) {
      const low = this.mode === 5
      const finalLow = low && this.count === 3
      const payload: u8 = finalLow ? byte & 0x1f : byte & 0x7f
      this.wide |= (payload as u64) << (this.shift as u64)
      this.shift += 7
      this.count += 1
      const more = (byte & 0x80) !== 0
      if (low) {
        if (more) {
          if (this.count >= 4) await this.fail('overlong', at)
          return
        }
      } else {
        if (this.count < 4) {
          if (!more) await this.fail('malformed', at)
          return
        }
        if (this.count === 4 && !more) {
          if ((byte & 0x7e) !== 0) {
            await this.fail('malformed', at)
            return
          }
        } else if (this.count === 4 || this.count === 5) {
          if (!more) await this.fail('malformed', at)
          return
        } else if (more || (byte & 0x78) !== 0) {
          await this.fail(more ? 'overlong' : 'malformed', at)
          return
        }
      }
      const first = this.start
      if (first === undefined) return
      const placed = over(first.span.start, at.span.end, at.settled)
      if (low)
        await this.out.emit(
          {
            kind: 'global-low',
            fragment: this.wide as u32,
            significantBits: this.count === 4 ? 26 : this.count * 7,
            wrap: finalLow && (byte & 0x40) !== 0,
            clockChange: finalLow && (byte & 0x20) !== 0,
          },
          placed,
        )
      else
        await this.out.emit(
          { kind: 'global-high', fragment: this.wide, timestampBits: this.count === 4 ? 48 : 64 },
          placed,
        )
      this.mode = 0
      this.start = undefined
      return
    }
    if (this.mode === 1) {
      this.value |= (byte as u32) << this.shift
      this.shift += 8
      this.remaining -= 1
      if (this.remaining === 0) await this.finishSource(at)
      return
    }
    if (this.mode === 2 || this.mode === 3) {
      const finalExtension = this.mode === 3 && this.count === 3
      this.value |= ((finalExtension ? byte : byte & 0x7f) as u32) << this.shift
      this.shift += 7
      this.count += 1
      if (!finalExtension && (byte & 0x80) !== 0) {
        if (this.count >= 4) await this.fail('overlong', at)
        return
      }
      const first = this.start
      if (first === undefined) return
      const placed = over(first.span.start, at.span.end, at.settled)
      if (this.mode === 2)
        await this.out.emit(
          { kind: 'timestamp', delta: this.value, control: (this.header >> 4) & 3 },
          placed,
        )
      else {
        const hardware = (this.header & 4) !== 0
        await this.out.emit({ kind: 'extension', hardware, value: this.value }, placed)
        if (!hardware) this.page = undefined
      }
      this.mode = 0
      this.start = undefined
      return
    }
    if (byte === 0) {
      if (this.zeros === 0) this.zeroAt = at
      if (this.zeros < 255) this.zeros += 1
      return
    }
    if (byte === 0x80 && this.zeros >= 5) {
      const zero = this.zeroAt
      if (zero !== undefined)
        await this.out.emit({ kind: 'sync' }, over(zero.span.start, at.span.end, at.settled))
      this.zeros = 0
      this.zeroAt = undefined
      this.aligned = true
      return
    }
    if (this.zeros > 0) {
      const zero = this.zeroAt
      this.zeros = 0
      this.zeroAt = undefined
      if (this.aligned && zero !== undefined) {
        await this.fail('bad-sync', over(zero.span.start, at.span.end, at.settled))
        return
      }
    }
    if (!this.aligned) return
    if (byte === 0x70) {
      this.page = undefined
      await this.out.emit({ kind: 'overflow' }, at)
      return
    }
    if (byte === 0x94 || byte === 0xb4) {
      this.header = byte
      this.mode = byte === 0x94 ? 5 : 6
      this.start = at
      this.wide = 0
      this.shift = 0
      this.count = 0
      return
    }
    const size: u8 = byte & 3
    if (size !== 0) {
      this.header = byte
      this.mode = 1
      this.size = size === 3 ? 4 : size
      this.remaining = this.size
      this.value = 0
      this.shift = 0
      this.start = at
      return
    }
    if ((byte & 0x0f) === 0 && byte >= 0x10 && byte <= 0x60) {
      await this.out.emit({ kind: 'timestamp', delta: byte >> 4, control: 0 }, at)
      return
    }
    if ((byte & 0xcf) === 0xc0) {
      this.mode = 2
      this.header = byte
      this.start = at
      this.value = 0
      this.shift = 0
      this.count = 0
      return
    }
    if ((byte & 0x0b) === 8) {
      this.header = byte
      this.value = (byte >> 4) & 7
      if ((byte & 0x80) === 0) {
        const hardware = (byte & 4) !== 0
        await this.out.emit({ kind: 'extension', hardware, value: this.value }, at)
        if (!hardware) this.page = this.value
        return
      }
      this.mode = 3
      this.start = at
      this.shift = 3
      this.count = 0
      return
    }
    this.header = byte
    this.mode = 4
    this.start = at
    await this.fail('unsupported-header', at)
  }
  override async run(): Promise<void> {
    for (;;) {
      const received = await this.input.next()
      const event = received.event
      if (event.kind === 'error') {
        await this.fail('input-error', received.at)
        continue
      }
      if (event.kind === 'byte') await this.consume(event.value, received.at)
    }
  }
}
export const armItm = defineStreamDecoder({
  name: 'arm-itm',
  category: 'device',
  config: { defaults: { startAligned: false } },
  input: traceByteEvents,
  output: armItmEvents,
  locales: {
    'en-US': {
      name: 'Arm ITM / DWT',
      summary:
        'Unformatted SWO bytes, source packets and local timestamps; synchronize before decoding',
      fields: { startAligned: 'Assume a packet boundary at the start' },
      events: {
        sync: 'Synchronization',
        overflow: 'Overflow',
        software: 'Instrumentation',
        hardware: 'DWT source',
        'event-counter': 'Event counter',
        exception: 'Exception',
        pc: 'PC sample',
        sleep: 'Sleep',
        timestamp: 'Local timestamp',
        'global-low': 'Global timestamp low fragment',
        'global-high': 'Global timestamp high fragment',
        extension: 'Extension',
        error: 'Error',
      },
      eventFields: {
        fragment: 'Timestamp fragment',
        significantBits: 'Significant low bits',
        timestampBits: 'Timestamp width',
        wrap: 'Upper timestamp changed',
        clockChange: 'Clock changed',
        port: 'Port',
        page: 'Port page',
        bytes: 'Bytes',
        value: 'Value',
        source: 'Source',
        flags: 'Flags',
        exception: 'Exception number',
        action: 'Exception action',
        delta: 'Timestamp delta',
        control: 'Timestamp control',
        hardware: 'Hardware source',
        header: 'Header',
        reason: 'Reason',
      },
    },
    'zh-CN': {
      name: 'Arm ITM / DWT',
      summary: '未格式化 SWO 字节、源数据包及局部时间戳；同步后解码',
      fields: { startAligned: '假定起点位于数据包边界' },
      events: {
        sync: '同步',
        overflow: '溢出',
        software: '软件跟踪',
        hardware: 'DWT 源',
        'event-counter': '事件计数器',
        exception: '异常',
        pc: 'PC 采样',
        sleep: '休眠',
        timestamp: '局部时间戳',
        'global-low': '全局时间戳低位片段',
        'global-high': '全局时间戳高位片段',
        extension: '扩展',
        error: '错误',
      },
      eventFields: {
        fragment: '时间戳片段',
        significantBits: '有效低位数',
        timestampBits: '时间戳宽度',
        wrap: '时间戳高位已变化',
        clockChange: '时钟已变化',
        port: '端口',
        page: '端口页',
        bytes: '字节数',
        value: '值',
        source: '源编号',
        flags: '标志',
        exception: '异常编号',
        action: '异常动作',
        delta: '时间戳增量',
        control: '时间戳控制',
        hardware: '硬件源',
        header: '头部',
        reason: '原因',
      },
    },
  },
  decoder: ArmItm,
})
