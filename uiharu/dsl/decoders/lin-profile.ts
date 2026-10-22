// SPDX-License-Identifier: Apache-2.0
/** LIN 1.x/2.x framing and checksum profile over UART, authored from LIN 2.2A
 * Protocol Specification §2.3 and LIN 1.3 identifier-derived lengths.
 * https://community.nxp.com/pwmxy87654/attachments/pwmxy87654/16-bit/18786/3/LIN_Specification_Package_2.2A.pdf
 * Idle completion is an explicit analysis threshold, not an inferred LDF length.
 */
import type { Provenance, Range, Tick, bool, u8, u16, u32, u64 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, mulDiv, mulDivCeil, over } from 'uiharu'
import { uartEvents, type UartEvent } from './uart'
export type Version = '1' | '2'
export interface Config {
  baud: u32
  version: Version
  minBreakBits: u8
  idleBits: u8
  lengthById: bool
}
export type Reason = 'sync' | 'parity' | 'framing' | 'checksum' | 'too-short' | 'too-long'
export type EndReason = 'complete' | 'no-response' | 'error' | 'aborted'
export type Event =
  | { kind: 'header'; frame: u64; pid: u8; id: u8; pidOk: bool }
  | { kind: 'data'; frame: u64; offset: u8; value: u8 }
  | { kind: 'checksum'; frame: u64; value: u8; classicOk: bool; enhancedOk: bool; selectedOk: bool }
  | { kind: 'error'; frame: u64; reason: Reason }
  | { kind: 'end'; frame: u64; reason: EndReason; idle: bool }
export const events = eventSchema<Event>('lin-profile.events', 1).roles({
  header: 'address',
  data: 'data',
  checksum: { role: 'check', errorWhen: { selectedOk: false } },
  error: 'error',
  end: 'framing',
})
function sum(sum: u16, byte: u8): u16 {
  const value: u16 = sum + byte
  return (value & 255) + (value >> 8)
}
function pidOk(pid: u8): bool {
  const p0: u8 = ((pid >> 0) ^ (pid >> 1) ^ (pid >> 2) ^ (pid >> 4)) & 1
  const p1: u8 = 1 ^ (((pid >> 1) ^ (pid >> 3) ^ (pid >> 4) ^ (pid >> 5)) & 1)
  return pid >> 6 === (p0 | (p1 << 1))
}
class LinProfile extends StreamDecoder<Config, UartEvent, Event> {
  state: u8 = 0
  frame: u64 = 0
  first: Provenance | undefined = undefined
  pid: u8 = 0
  data: u8 = 0
  sum: u16 = 0
  enhanced: u16 = 0
  expected: u8 = 0
  pending: u8 | undefined = undefined
  pendingAt: Provenance | undefined = undefined
  deadline: Tick | undefined = undefined
  failed = false
  override hold(): Tick | undefined {
    const first = this.first
    if (first !== undefined) return first.span.start
    return this.pendingAt?.span.start
  }
  clear(): void {
    this.state = 0
    this.first = undefined
    this.pending = undefined
    this.pendingAt = undefined
    this.deadline = undefined
  }
  async error(reason: Reason, at: Provenance): Promise<void> {
    this.failed = true
    await this.out.emit({ kind: 'error', frame: this.frame, reason }, at)
  }
  async dataByte(value: u8, at: Provenance): Promise<void> {
    await this.out.emit({ kind: 'data', frame: this.frame, offset: this.data, value }, at)
    this.data += 1
    this.sum = sum(this.sum, value)
    this.enhanced = sum(this.enhanced, value)
  }
  async close(tick: Tick, settled: Tick, aborted: bool, idle: bool): Promise<void> {
    if (this.state === 0) return
    if (this.state !== 3) {
      this.clear()
      return
    }
    const pending = this.pending
    const pendingAt = this.pendingAt
    this.pending = undefined
    this.pendingAt = undefined
    let reason: EndReason = aborted ? 'aborted' : 'complete'
    if (pending === undefined) reason = aborted ? 'aborted' : 'no-response'
    else if (pendingAt !== undefined) {
      const at = over(pendingAt.span.start, pendingAt.span.end, settled)
      if (aborted) await this.dataByte(pending, at)
      else if (this.data === 0 || (this.config.lengthById && this.data !== this.expected)) {
        await this.dataByte(pending, at)
        await this.error('too-short', at)
      } else {
        const classicOk = sum(this.sum, pending) === 255
        const enhancedOk = sum(this.enhanced, pending) === 255
        const id: u8 = this.pid & 63
        const selectedOk =
          this.config.version === '1' || id === 60 || id === 61 ? classicOk : enhancedOk
        await this.out.emit(
          {
            kind: 'checksum',
            frame: this.frame,
            value: pending,
            classicOk,
            enhancedOk,
            selectedOk,
          },
          at,
        )
        if (!selectedOk) await this.error('checksum', at)
      }
    }
    if (!aborted && this.failed) reason = 'error'
    await this.out.emit({ kind: 'end', frame: this.frame, reason, idle }, over(tick, tick, settled))
    this.clear()
  }
  async idle(tick: Tick): Promise<void> {
    const deadline = this.deadline
    if (deadline !== undefined && tick >= deadline)
      await this.close(deadline, deadline, false, true)
  }
  override async onProgress(tick: Tick): Promise<void> {
    await this.idle(tick)
  }
  override async onGap(range: Range): Promise<void> {
    await this.close(range.start, range.start, true, false)
    this.clear()
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.idle(tick)
    await this.close(tick, tick, true, false)
    this.clear()
  }
  override async run(): Promise<void> {
    for (;;) {
      const received = await this.input.next()
      const e = received.event
      const at = received.at
      await this.idle(at.span.start)
      if (e.kind === 'break') {
        await this.close(at.span.start, at.settled, false, false)
        if (e.ticks >= mulDiv(this.rate, this.config.minBreakBits, this.config.baud)) {
          this.frame += 1
          this.state = 1
          this.first = at
          this.data = 0
          this.sum = 0
          this.failed = false
        }
      } else if (e.kind === 'frame' && this.state !== 0) {
        const byte: u8 = e.value as u8
        if (this.state === 1) {
          if (byte !== 0x55 || !e.framingOk) {
            await this.error('sync', at)
            this.clear()
            continue
          }
          this.state = 2
        } else if (this.state === 2) {
          this.pid = byte
          this.enhanced = byte
          const first = this.first
          this.first = undefined
          const valid = pidOk(byte)
          await this.out.emit(
            { kind: 'header', frame: this.frame, pid: byte, id: byte & 63, pidOk: valid },
            first === undefined ? at : over(first.span.start, at.span.end, at.settled),
          )
          if (!valid) await this.error('parity', at)
          if (!e.framingOk) await this.error('framing', at)
          this.expected = (byte & 0x30) < 0x20 ? 2 : (byte & 0x30) === 0x20 ? 4 : 8
          this.state = 3
        } else {
          const pending = this.pending
          const pendingAt = this.pendingAt
          this.pending = undefined
          this.pendingAt = undefined
          if (pending !== undefined && pendingAt !== undefined)
            await this.dataByte(pending, pendingAt)
          if (this.data > 8) {
            await this.error('too-long', at)
            await this.close(at.span.end, at.settled, false, false)
            continue
          }
          this.pending = byte
          this.pendingAt = at
          if (!e.framingOk) await this.error('framing', at)
          if (this.config.lengthById && this.data === this.expected) {
            await this.close(at.span.end, at.settled, false, false)
            continue
          }
        }
        if (this.state === 3)
          this.deadline =
            at.span.end + mulDivCeil(this.rate, this.config.idleBits, this.config.baud)
      }
    }
  }
}
export const linProfile = defineStreamDecoder({
  name: 'lin-profile',
  category: 'bus',
  config: {
    defaults: { baud: 19200, version: '2', minBreakBits: 11, idleBits: 20, lengthById: false },
    validate: (config) =>
      config.baud === 0
        ? 'decode.config.baud'
        : config.minBreakBits < 9 || config.minBreakBits > 13
          ? 'decode.config.linBreak'
          : config.idleBits < 10
            ? 'decode.config.linIdle'
            : undefined,
  },
  input: uartEvents,
  requires: (config) => ({
    baud: config.baud,
    dataBits: 8,
    parity: 'none',
    stopBits: 'one',
    bitOrder: 'lsb-first',
  }),
  output: events,
  locales: {
    'en-US': {
      name: 'LIN version and idle profile',
      summary: 'Selected classic/enhanced checksum with explicit idle response completion',
      fields: {
        baud: 'Baud',
        version: 'LIN version',
        minBreakBits: 'Minimum BREAK bits',
        idleBits: 'Response idle threshold (bits)',
        lengthById: 'Use LIN 1.x identifier length',
      },
      values: { version: { '1': 'LIN 1.x', '2': 'LIN 2.x' } },
      events: {
        header: 'Header',
        data: 'Data',
        checksum: 'Checksum',
        error: 'Protocol error',
        end: 'End',
      },
      eventFields: {
        frame: 'Frame',
        pid: 'Protected identifier',
        id: 'Identifier',
        pidOk: 'PID parity OK',
        offset: 'Offset',
        value: 'Value',
        classicOk: 'Classic checksum OK',
        enhancedOk: 'Enhanced checksum OK',
        selectedOk: 'Selected checksum OK',
        reason: 'Reason',
        idle: 'Ended at idle threshold',
      },
    },
    'zh-CN': {
      name: 'LIN版本和空闲格式',
      summary: '指定经典或增强校验和，并以明确空闲阈值结束响应',
      fields: {
        baud: '波特率',
        version: 'LIN版本',
        minBreakBits: '最少BREAK位数',
        idleBits: '响应空闲阈值（位）',
        lengthById: '使用LIN 1.x标识符长度',
      },
      values: { version: { '1': 'LIN 1.x', '2': 'LIN 2.x' } },
      events: { header: '帧头', data: '数据', checksum: '校验和', error: '协议错误', end: '结束' },
      eventFields: {
        frame: '帧',
        pid: '受保护标识符',
        id: '标识符',
        pidOk: 'PID奇偶校验正确',
        offset: '偏移',
        value: '值',
        classicOk: '经典校验和正确',
        enhancedOk: '增强校验和正确',
        selectedOk: '指定校验和正确',
        reason: '原因',
        idle: '达到空闲阈值结束',
      },
    },
  },
  decoder: LinProfile,
})
