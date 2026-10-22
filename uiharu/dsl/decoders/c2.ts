// SPDX-License-Identifier: Apache-2.0
/** Silicon Labs C2 transport, AN127 sections1.2–1.5. Independent implementation
 * from https://www.silabs.com/documents/public/application-notes/AN127.pdf . */
import type { Channel, Range, Tick, Provenance, bool, u8, u16, u64 } from 'uiharu'
import {
  LogicDecoder,
  defineLogicDecoder,
  eventSchema,
  mask,
  max,
  atLeast,
  mulDivCeil,
  one,
  over,
  join,
  fromBool,
} from 'uiharu'
export interface Config {
  clock: Channel
  data: Channel
  maxWaitCycles: u16
  startAligned: bool
}
export type Command = 'read-data' | 'write-data' | 'read-address' | 'write-address'
export type Phase = 'unsynced' | 'start' | 'instruction' | 'length' | 'wait' | 'data' | 'stop'
export type End = 'complete' | 'aborted' | 'wait-limit'
export type Event =
  | { kind: 'reset'; ticks: u64 }
  | { kind: 'instruction'; op: u64; command: Command }
  | { kind: 'length'; op: u64; bytes: u8 }
  | { kind: 'wait'; op: u64; cycles: u16; ready: bool }
  | { kind: 'byte'; op: u64; index: u8; value: u8; address: bool; read: bool }
  | { kind: 'end'; op: u64; reason: End }
export const events = eventSchema<Event>('c2.events', 1).roles({
  reset: 'control',
  instruction: 'command',
  length: 'control',
  wait: 'handshake',
  byte: 'data',
  end: 'framing',
})
function validate(c: Config): string | undefined {
  if (c.clock >= 64 || c.data >= 64 || c.clock === c.data) return 'c2.config.channels'
  if (c.maxWaitCycles === 0) return 'c2.config.wait'
  return undefined
}
class C2 extends LogicDecoder<Config, Event> {
  readonly watch = mask(this.config.clock)
  phase: Phase = this.config.startAligned ? 'start' : 'unsynced'
  low: Tick | undefined = undefined
  first: Provenance | undefined = undefined
  byteFirst: Provenance | undefined = undefined
  op: u64 = 0
  instruction: u8 = 0
  bits: u8 = 0
  value: u8 = 0
  bytes: u8 = 0
  index: u8 = 0
  waits: u16 = 0
  override hold(): Tick | undefined {
    const first = this.first
    if (first !== undefined) return first.span.start
    return this.low
  }
  override async onGap(range: Range): Promise<void> {
    await this.cut(range.start)
    this.low = undefined
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
    this.low = undefined
  }
  async cut(tick: Tick): Promise<void> {
    if (this.first !== undefined)
      await this.out.emit({ kind: 'end', op: this.op, reason: 'aborted' }, over(tick, tick, tick))
    this.phase = 'unsynced'
    this.first = undefined
    this.byteFirst = undefined
  }
  async finish(reason: End, at: Provenance): Promise<void> {
    await this.out.emit({ kind: 'end', op: this.op, reason }, at)
    this.first = undefined
    this.byteFirst = undefined
    this.phase = reason === 'complete' ? 'start' : 'unsynced'
  }
  async bit(bit: u8, at: Provenance): Promise<void> {
    if (this.phase === 'unsynced') return
    if (this.phase === 'start') {
      this.op += 1
      this.first = at
      this.phase = 'instruction'
      this.bits = 0
      this.value = 0
      return
    }
    if (this.phase === 'stop') {
      await this.finish('complete', at)
      return
    }
    if (this.phase === 'wait') {
      this.waits += 1
      if (bit !== 0) {
        await this.out.emit({ kind: 'wait', op: this.op, cycles: this.waits, ready: true }, at)
        this.bits = 0
        this.value = 0
        this.phase = this.instruction === 0 ? 'data' : 'stop'
      } else if (this.waits === this.config.maxWaitCycles) {
        await this.out.emit({ kind: 'wait', op: this.op, cycles: this.waits, ready: false }, at)
        await this.finish('wait-limit', at)
      }
      return
    }
    if (this.bits === 0) this.byteFirst = at
    this.value |= bit << (this.bits as u64)
    this.bits += 1
    if (this.phase === 'instruction' && this.bits === 2) {
      this.instruction = this.value
      const command: Command =
        this.value === 0
          ? 'read-data'
          : this.value === 1
            ? 'write-data'
            : this.value === 2
              ? 'read-address'
              : 'write-address'
      const first = this.byteFirst
      const placed = first === undefined ? at : join(first, at)
      await this.out.emit({ kind: 'instruction', op: this.op, command }, placed)
      this.bits = 0
      this.value = 0
      this.index = 0
      this.waits = 0
      if (this.instruction >= 2) {
        this.bytes = 1
        this.phase = 'data'
      } else this.phase = 'length'
      return
    }
    if (this.phase === 'length' && this.bits === 2) {
      this.bytes = this.value + 1
      await this.out.emit({ kind: 'length', op: this.op, bytes: this.bytes }, at)
      this.bits = 0
      this.value = 0
      this.phase = this.instruction === 0 ? 'wait' : 'data'
      return
    }
    if (this.phase === 'data' && this.bits === 8) {
      const first = this.byteFirst
      const placed = first === undefined ? at : join(first, at)
      await this.out.emit(
        {
          kind: 'byte',
          op: this.op,
          index: this.index,
          value: this.value,
          address: this.instruction >= 2,
          read: (this.instruction & 1) === 0,
        },
        placed,
      )
      this.bits = 0
      this.value = 0
      this.index += 1
      if (this.index === this.bytes) this.phase = this.instruction === 1 ? 'wait' : 'stop'
    }
  }
  override async run(): Promise<void> {
    await this.signal.sample()
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      if (!edge.levels.high(this.config.clock)) {
        this.low = edge.tick
        continue
      }
      const low = this.low
      this.low = undefined
      if (low !== undefined && edge.tick - low >= mulDivCeil(this.rate, 20, 1000000)) {
        await this.cut(edge.tick)
        await this.out.emit(
          { kind: 'reset', ticks: edge.tick - low },
          over(low, edge.tick + 1, edge.tick),
        )
        this.phase = 'start'
        continue
      }
      await this.bit(fromBool(edge.levels.high(this.config.data)), one(edge.tick))
    }
  }
}
export const c2 = defineLogicDecoder({
  name: 'c2',
  category: 'bus',
  config: { defaults: { clock: 0, data: 1, maxWaitCycles: 1024, startAligned: false }, validate },
  requires: (config) => ({
    channels: atLeast(max(config.clock, config.data) + 1),
    rateHz: atLeast(1),
  }),
  output: events,
  props: () => ({}),
  locales: {
    'en-US': {
      name: 'Silicon Labs C2',
      summary: 'C2 reset, address/data frames and bounded variable WAIT',
      fields: {
        clock: 'C2CK',
        data: 'C2D',
        maxWaitCycles: 'Maximum WAIT cycles',
        startAligned: 'Start at a known START boundary',
      },
      events: {
        reset: 'Reset',
        instruction: 'Instruction',
        length: 'Data length',
        wait: 'WAIT',
        byte: 'Byte',
        end: 'End',
      },
      eventFields: {
        ticks: 'Reset low ticks',
        op: 'Operation',
        command: 'Instruction',
        bytes: 'Byte count',
        cycles: 'WAIT cycles',
        ready: 'Ready',
        index: 'Byte index',
        value: 'Value',
        address: 'Address register',
        read: 'Read',
        reason: 'End reason',
      },
    },
    'zh-CN': {
      name: 'Silicon Labs C2',
      summary: 'C2复位、地址/数据帧及有界等待',
      fields: {
        clock: 'C2CK',
        data: 'C2D',
        maxWaitCycles: '最大等待周期',
        startAligned: '从已知START边界开始',
      },
      events: {
        reset: '复位',
        instruction: '指令',
        length: '数据长度',
        wait: '等待',
        byte: '字节',
        end: '结束',
      },
      eventFields: {
        ticks: '复位低电平采样数',
        op: '操作号',
        command: '指令',
        bytes: '字节数',
        cycles: '等待周期',
        ready: '就绪',
        index: '字节索引',
        value: '数值',
        address: '地址寄存器',
        read: '读取',
        reason: '结束原因',
      },
    },
  },
  decoder: C2,
})
