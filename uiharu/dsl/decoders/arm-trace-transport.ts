// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Integer assertions select IR widths and truncation; TypeScript aliases alone do not encode these semantics. */

/** CoreSight formatter: Arm IHI0029D D4, fixed sixteen-byte frames.
 * https://documentation-service.arm.com/static/5f9009d5f86e16515cdc0417?token=
 * Select one trace ID; preserve byte/error transport rather than invent UART frames.
 */
import type { bool, u8, u16, u32, u64, Tick, Range, Provenance } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
import { uartEvents, type UartEvent } from './uart'
export type TraceByteError = 'break' | 'framing' | 'parity' | 'alignment' | 'truncated'
export type TraceByteEvent = { kind: 'byte'; value: u8 } | { kind: 'error'; reason: TraceByteError }
export const traceByteEvents = eventSchema<TraceByteEvent>('trace-byte.events', 1).roles({
  byte: 'data',
  error: 'error',
})
export type TraceFormat = 'raw' | 'tpiu'
export interface ArmTraceTransportConfig {
  format: TraceFormat
  traceId: u8
  startAligned: bool
}
class ArmTraceTransport extends StreamDecoder<ArmTraceTransportConfig, UartEvent, TraceByteEvent> {
  aligned = this.config.startAligned
  id: u8 | undefined = undefined
  rolling: u32 = 0
  half: u8 | undefined = undefined
  halfAt: Provenance | undefined = undefined
  syncCandidate = false
  count: u8 = 0
  low: u64 = 0
  high: u64 = 0
  start: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    const first = this.start
    if (first !== undefined) return first.span.start
    const half = this.halfAt
    if (half !== undefined) return half.span.start
    return undefined
  }
  clear(): void {
    this.half = undefined
    this.halfAt = undefined
    this.syncCandidate = false
    this.count = 0
    this.low = 0
    this.high = 0
    this.start = undefined
  }
  async error(at: Provenance): Promise<void> {
    const first = this.start
    await this.out.emit(
      { kind: 'error', reason: 'truncated' },
      first === undefined ? at : over(first.span.start, at.span.end, at.settled),
    )
    this.clear()
    this.id = undefined
    this.aligned = false
    this.rolling = 0
  }
  override async onGap(range: Range): Promise<void> {
    if (this.count !== 0 || this.half !== undefined || this.syncCandidate)
      await this.error(over(range.start, range.start, range.start))
    this.clear()
    this.id = undefined
    this.aligned = false
    this.rolling = 0
  }
  override async onEnd(tick: Tick): Promise<void> {
    if (this.count !== 0 || this.half !== undefined || this.syncCandidate)
      await this.error(over(tick, tick, tick))
    this.clear()
  }
  byte(index: u8): u8 {
    if (index < 8) return ((this.low >> ((index * 8) as u64)) & 255) as u8
    return ((this.high >> (((index - 8) * 8) as u64)) & 255) as u8
  }
  async data(byte: u8, at: Provenance): Promise<void> {
    if (this.id === this.config.traceId) await this.out.emit({ kind: 'byte', value: byte }, at)
  }
  async frameByte(byte: u8, at: Provenance): Promise<void> {
    if (this.count === 0 && this.start === undefined) this.start = at
    if (this.count < 15) {
      if (this.count < 8) this.low |= (byte as u64) << ((this.count * 8) as u64)
      else this.high |= (byte as u64) << (((this.count - 8) * 8) as u64)
      this.count += 1
      return
    }
    const first = this.start
    if (first === undefined) return
    const placed = over(first.span.start, at.span.end, at.settled)
    let index: u8 = 0
    while (index < 8) {
      const mixed = this.byte(index * 2)
      const flag = (byte >> index) & 1
      if ((mixed & 1) !== 0) {
        const next: u8 = mixed >> 1
        if (index === 7) {
          this.id = next
        } else {
          if (flag === 0) this.id = next
          await this.data(this.byte(index * 2 + 1), placed)
          if (flag !== 0) this.id = next
        }
      } else {
        await this.data((mixed & 0xfe) | flag, placed)
        if (index !== 7) await this.data(this.byte(index * 2 + 1), placed)
      }
      index += 1
    }
    this.count = 0
    this.low = 0
    this.high = 0
    this.start = undefined
  }
  async word(word: u16, at: Provenance): Promise<void> {
    if (this.syncCandidate) {
      this.syncCandidate = false
      if (word === 0x7fff) {
        if (this.count !== 0) {
          const first = this.start
          await this.out.emit(
            { kind: 'error', reason: 'alignment' },
            first === undefined ? at : over(first.span.start, at.span.end, at.settled),
          )
        }
        this.count = 0
        this.low = 0
        this.high = 0
        this.start = undefined
        this.id = undefined
        return
      }
      await this.frameByte(0xff, at)
      await this.frameByte(0xff, at)
    }
    if (word === 0xffff) {
      if (this.count === 0 && this.start === undefined) this.start = at
      this.syncCandidate = true
      return
    }
    if (word === 0x7fff) return
    await this.frameByte((word & 255) as u8, at)
    await this.frameByte((word >> 8) as u8, at)
  }
  async transportError(reason: TraceByteError, at: Provenance): Promise<void> {
    this.clear()
    this.id = undefined
    this.aligned = false
    this.rolling = 0
    await this.out.emit({ kind: 'error', reason }, at)
  }
  async consume(byte: u8, at: Provenance): Promise<void> {
    if (this.config.format === 'raw') {
      await this.out.emit({ kind: 'byte', value: byte }, at)
      return
    }
    if (!this.aligned) {
      this.rolling = (this.rolling >> 8) | ((byte as u32) << 24)
      if (this.rolling === 0x7fffffff) {
        this.clear()
        this.aligned = true
        this.id = undefined
      }
      return
    }
    const half = this.half
    if (half === undefined) {
      this.half = byte
      this.halfAt = at
      return
    }
    const first = this.halfAt
    this.half = undefined
    this.halfAt = undefined
    await this.word(
      (half as u16) | ((byte as u16) << 8),
      first === undefined ? at : over(first.span.start, at.span.end, at.settled),
    )
  }
  override async run(): Promise<void> {
    for (;;) {
      const item = await this.input.next()
      const event = item.event
      if (event.kind === 'break') await this.transportError('break', item.at)
      if (event.kind === 'frame') {
        if (!event.framingOk) await this.transportError('framing', item.at)
        else if (event.parity === 'error') await this.transportError('parity', item.at)
        else await this.consume(event.value as u8, item.at)
      }
    }
  }
}
function validate(config: ArmTraceTransportConfig): string | undefined {
  if (config.traceId < 1 || config.traceId > 111) return 'decode.config.traceId'
  return undefined
}
export const armTraceTransport = defineStreamDecoder({
  name: 'arm-trace-transport',
  category: 'device',
  config: { defaults: { format: 'raw', traceId: 1, startAligned: false }, validate },
  input: uartEvents,
  requires: () => ({ dataBits: 8, bitOrder: 'lsb-first' }),
  output: traceByteEvents,
  locales: {
    'en-US': {
      name: 'Arm trace transport',
      summary: 'Raw SWO bytes or one CoreSight TPIU formatter trace source',
      fields: {
        format: 'Trace transport format',
        traceId: 'Trace source ID',
        startAligned: 'Assume a formatter frame boundary at the start',
      },
      values: { format: { raw: 'Raw trace bytes', tpiu: 'TPIU formatted stream' } },
      events: { byte: 'Trace byte', error: 'Transport error' },
      eventFields: { value: 'Value', reason: 'Reason' },
    },
    'zh-CN': {
      name: 'Arm 跟踪传输',
      summary: '原始 SWO 字节或 CoreSight TPIU 格式器跟踪源',
      fields: {
        format: '跟踪传输格式',
        traceId: '跟踪源 ID',
        startAligned: '假定起点位于格式帧边界',
      },
      values: { format: { raw: '原始跟踪字节', tpiu: 'TPIU 格式化流' } },
      events: { byte: '跟踪字节', error: '传输错误' },
      eventFields: { value: '值', reason: '原因' },
    },
  },
  decoder: ArmTraceTransport,
})
