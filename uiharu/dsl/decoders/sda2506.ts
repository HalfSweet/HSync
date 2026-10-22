// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */
/** Siemens SDA2506-5/-6 serial EEPROM, original 1994 Semiconductor Group data sheet:
 * https://dtsheet.com/doc/196319/infineon-sda2506-5 . Input shifts LSB first
 * while CE is high; CE falling latches the last 8/16-bit instruction. CE-low
 * read data is sampled on rising clocks after the initial load clock. Program
 * events describe requested erase/write, not a verified memory change. No TP2
 * total erase or analog programming-pulse duration validation. Bounded 16-bit latch. */
import type { Channel, Range, Tick, bool, u8, u16, u32, u64 } from 'uiharu'
import {
  LogicDecoder,
  at,
  atLeast,
  defineLogicDecoder,
  eventSchema,
  fromBool,
  mask,
  max,
  one,
  over,
} from 'uiharu'
export type Operation = 'read' | 'erase' | 'write'
export type Event =
  | {
      kind: 'command'
      tx: u64
      operation: Operation
      address: u8
      value: u8 | undefined
      reused: bool
    }
  | { kind: 'data'; tx: u64; address: u8; value: u8 }
  | { kind: 'end'; tx: u64; aborted: bool; bits: u8 }
export const events = eventSchema<Event>('sda2506.events', 1).roles({
  command: 'command',
  data: 'data',
  end: 'framing',
})
export interface Config {
  clk: Channel
  data: Channel
  ce: Channel
}
function validate(c: Config): string | undefined {
  if (max(max(c.clk, c.data), c.ce) >= 64) return 'sda2506.config.channel'
  if (c.clk === c.data || c.clk === c.ce || c.data === c.ce)
    return 'sda2506.config.duplicateChannel'
  return undefined
}
class Memory extends LogicDecoder<Config, Event> {
  readonly watch = mask(this.config.clk, this.config.ce)
  latch: u16 = 0
  inputBits: u32 = 0
  first: Tick = 0
  latched: u16 = 0
  known = false
  pending = false
  reused = false
  tx: u64 = 0
  open = false
  reading = false
  loaded = false
  address: u8 = 0
  bits: u8 = 0
  value: u8 = 0
  dataFirst: Tick = 0
  override hold(): Tick | undefined {
    return this.inputBits > 0 || this.pending
      ? this.first
      : this.reading && this.bits > 0
        ? this.dataFirst
        : undefined
  }
  async close(tick: Tick, aborted: bool): Promise<void> {
    if (this.open) {
      this.open = false
      await this.out.emit(
        { kind: 'end', tx: this.tx, aborted, bits: this.bits },
        aborted ? at(tick) : one(tick),
      )
    }
    this.pending = false
    this.reading = false
    this.bits = 0
  }
  override async onGap(range: Range): Promise<void> {
    await this.close(range.start, true)
    this.known = false
    this.inputBits = 0
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.close(tick, true)
    this.inputBits = 0
  }
  override async run(): Promise<void> {
    await this.signal.sample()
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      if (edge.changed.has(this.config.ce)) {
        if (edge.levels.high(this.config.ce)) {
          await this.close(edge.tick, false)
          this.inputBits = 0
          this.latch = 0
        } else {
          this.reused = this.inputBits === 0
          if (this.reused) this.first = edge.tick
          if (this.inputBits === 8 || this.inputBits === 16) {
            this.latched = this.latch
            this.known = true
          } else if (this.inputBits !== 0) this.known = false
          this.inputBits = 0
          if (this.known) {
            this.tx += 1
            this.open = true
            this.bits = 0
            this.value = 0
            this.loaded = false
            this.pending = true
            this.reading = (this.latched & 0x8000) === 0
            this.address = ((this.latched >> 8) & 127) as u8
            if (this.reading) {
              await this.out.emit(
                {
                  kind: 'command',
                  tx: this.tx,
                  operation: 'read',
                  address: this.address,
                  value: undefined,
                  reused: this.reused,
                },
                over(this.first, edge.tick + 1, edge.tick),
              )
              this.pending = false
            }
          }
        }
        continue
      }
      if (edge.levels.high(this.config.ce)) {
        if (!edge.levels.high(this.config.clk)) {
          if (this.inputBits === 0) this.first = edge.tick
          this.latch = (this.latch >> 1) | (fromBool(edge.levels.high(this.config.data)) << 15)
          if (this.inputBits < 17) this.inputBits += 1
        }
        continue
      }
      if (!this.open) continue
      if (this.pending && !edge.levels.high(this.config.clk)) {
        this.pending = false
        const erase = edge.levels.high(this.config.data)
        await this.out.emit(
          {
            kind: 'command',
            tx: this.tx,
            operation: erase ? 'erase' : 'write',
            address: this.address,
            value: erase ? undefined : ((this.latched & 255) as u8),
            reused: this.reused,
          },
          over(this.first, edge.tick + 1, edge.tick),
        )
      }
      if (this.reading && edge.levels.high(this.config.clk)) {
        if (!this.loaded) {
          this.loaded = true
          continue
        }
        if (this.bits === 0) this.dataFirst = edge.tick
        this.value |= fromBool(edge.levels.high(this.config.data)) << this.bits
        this.bits += 1
        if (this.bits === 8) {
          await this.out.emit(
            { kind: 'data', tx: this.tx, address: this.address, value: this.value },
            over(this.dataFirst, edge.tick + 1, edge.tick),
          )
          this.reading = false
        }
      }
    }
  }
}
export const memory = defineLogicDecoder({
  name: 'sda2506',
  category: 'device',
  config: { defaults: { clk: 0, data: 1, ce: 2 }, validate },
  requires: (config) => ({ channels: atLeast(max(max(config.clk, config.data), config.ce) + 1) }),
  output: events,
  props: () => ({ addressBits: 7, lsbFirst: true }),

  locales: {
    'en-US': {
      name: 'SDA2506',
      summary: 'SDA2506 shared-data read and erase/write request sequences',
      fields: { clk: 'Clock channel', data: 'Data', ce: 'Enable channel' },
      events: { command: 'Command', data: 'Data', end: 'End' },
      eventFields: {
        tx: 'Transaction',
        operation: 'Operation',
        address: 'Address',
        value: 'Value',
        reused: 'Reused',
        aborted: 'Aborted',
        bits: 'Bits',
      },
      values: {},
    },
    'zh-CN': {
      name: 'SDA2506',
      summary: '解析SDA2506共享数据线读取及擦写请求序列',
      fields: { clk: '时钟通道', data: '数据通道', ce: '使能通道' },
      events: { command: '命令', data: '数据通道', end: '结束' },
      eventFields: {
        tx: '事务号',
        operation: '操作',
        address: '地址',
        value: '数值',
        reused: '复用指令',
        aborted: '中断',
        bits: '位数',
      },
      values: {},
    },
  },
  decoder: Memory,
})
