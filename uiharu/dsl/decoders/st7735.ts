// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */
/** Sitronix st7735 four-wire serial write interface; D/C distinguishes command and data.
 * Original manufacturer specification: https://e2e.ti.com/cfs-file/__key/communityserver-discussions-components-files/908/ST7735S_5F00_v1.3.pdf .
 * Raw parameters, CASET/RASET windows and RGB565/RGB666 pixel values after observed
 * COLMOD. No framebuffer, readback, tearing/reset signal or parallel-bus inference. */
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
export type Event =
  | { kind: 'begin'; tx: u64; partial: bool }
  | { kind: 'command'; tx: u64; value: u8 }
  | { kind: 'parameter'; tx: u64; command: u8 | undefined; offset: u32; value: u8 }
  | { kind: 'window'; tx: u64; column: bool; first: u16; last: u16 }
  | { kind: 'pixel'; tx: u64; offset: u32; value: u32; bits: u8 }
  | { kind: 'end'; tx: u64; aborted: bool; droppedBits: u8 }
export const events = eventSchema<Event>('st7735.events', 1).roles({
  begin: 'framing',
  command: 'command',
  parameter: 'data',
  window: 'address',
  pixel: 'data',
  end: 'framing',
})
export interface Config {
  cs: Channel
  clk: Channel
  data: Channel
  dc: Channel
}
function validate(c: Config): string | undefined {
  if (max(max(c.cs, c.clk), max(c.data, c.dc)) >= 64) return 'st7735.config.channel'
  if (
    c.cs === c.clk ||
    c.cs === c.data ||
    c.cs === c.dc ||
    c.clk === c.data ||
    c.clk === c.dc ||
    c.data === c.dc
  )
    return 'st7735.config.duplicateChannel'
  return undefined
}
class Display extends LogicDecoder<Config, Event> {
  readonly watch = mask(this.config.cs, this.config.clk, this.config.dc)
  tx: u64 = 0
  open = false
  partial = false
  bits: u8 = 0
  value: u8 = 0
  first: Tick = 0
  dataByte = false
  command: u8 | undefined = undefined
  offset: u32 = 0
  argument: u32 = 0
  pixelBits: u8 = 0
  pixel: u32 = 0
  pixelBytes: u8 = 0
  pixelOffset: u32 = 0
  held: Tick | undefined = undefined
  override hold(): Tick | undefined {
    return this.held !== undefined ? this.held : this.bits > 0 ? this.first : undefined
  }
  override async onGap(range: Range): Promise<void> {
    await this.end(range.start, true)
    this.command = undefined
    this.pixelBits = 0
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.end(tick, true)
  }
  async end(tick: Tick, aborted: bool): Promise<void> {
    if (!this.open) return
    this.open = false
    await this.out.emit(
      { kind: 'end', tx: this.tx, aborted, droppedBits: this.bits },
      aborted ? at(tick) : one(tick),
    )
    if (this.bits > 0) {
      this.command = undefined
      this.pixelBytes = 0
      this.held = undefined
    }
    this.bits = 0
  }
  async begin(tick: Tick, partial: bool): Promise<void> {
    this.tx += 1
    this.open = true
    this.partial = partial
    this.bits = 0
    if (partial) {
      this.command = undefined
      this.pixelBytes = 0
      this.held = undefined
    }
    await this.out.emit({ kind: 'begin', tx: this.tx, partial }, one(tick))
  }
  async byte(tick: Tick): Promise<void> {
    const value = this.value
    const where = over(this.first, tick + 1, tick)
    if (!this.dataByte) {
      this.command = value
      this.offset = 0
      this.argument = 0
      this.pixel = 0
      this.pixelBytes = 0
      this.pixelOffset = 0
      this.held = undefined
      await this.out.emit({ kind: 'command', tx: this.tx, value }, where)
      return
    }
    await this.out.emit(
      { kind: 'parameter', tx: this.tx, command: this.command, offset: this.offset, value },
      where,
    )
    if (this.command === 0x3a && this.offset === 0) {
      this.pixelBits = (value & 7) === 5 ? 16 : (value & 7) === 6 ? 18 : 0
    }
    if ((this.command === 0x2a || this.command === 0x2b) && this.offset < 4) {
      if (this.offset === 0) this.held = this.first
      this.argument = (this.argument << 8) | value
      if (this.offset === 3) {
        await this.out.emit(
          {
            kind: 'window',
            tx: this.tx,
            column: this.command === 0x2a,
            first: (this.argument >> 16) as u16,
            last: (this.argument & 0xffff) as u16,
          },
          over(this.held ?? this.first, tick + 1, tick),
        )
        this.held = undefined
      }
    }
    if ((this.command === 0x2c || this.command === 0x3c) && this.pixelBits !== 0) {
      if (this.pixelBytes === 0) this.held = this.first
      this.pixel = (this.pixel << 8) | value
      this.pixelBytes += 1
      if (this.pixelBytes === (this.pixelBits === 16 ? 2 : 3)) {
        await this.out.emit(
          {
            kind: 'pixel',
            tx: this.tx,
            offset: this.pixelOffset,
            value: this.pixel,
            bits: this.pixelBits,
          },
          over(this.held ?? this.first, tick + 1, tick),
        )
        this.pixelOffset += 1
        this.pixelBytes = 0
        this.pixel = 0
        this.held = undefined
      }
    }
    this.offset += 1
  }
  override async run(): Promise<void> {
    const initial = await this.signal.sample()
    if (!initial.high(this.config.cs)) await this.begin(this.signal.position, true)
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      if (edge.changed.has(this.config.cs)) {
        if (!edge.levels.high(this.config.cs)) await this.begin(edge.tick, false)
        else await this.end(edge.tick, false)
        continue
      }
      if (!this.open || this.partial) continue
      if (edge.changed.has(this.config.dc) && this.bits > 0) {
        this.partial = true
        continue
      }
      if (edge.changed.has(this.config.clk) && edge.levels.high(this.config.clk)) {
        if (this.bits === 0) {
          this.first = edge.tick
          this.dataByte = edge.levels.high(this.config.dc)
        }
        this.value = (this.value << 1) | fromBool(edge.levels.high(this.config.data))
        this.bits += 1
        if (this.bits === 8) {
          this.bits = 0
          await this.byte(edge.tick)
          this.value = 0
        }
      }
    }
  }
}
export const display = defineLogicDecoder({
  name: 'st7735',
  category: 'device',
  config: { defaults: { cs: 0, clk: 1, data: 2, dc: 3 }, validate },
  requires: (config) => ({
    channels: atLeast(max(max(config.cs, config.clk), max(config.data, config.dc)) + 1),
  }),
  output: events,
  props: () => ({ fourWire: true }),

  locales: {
    'en-US': {
      name: 'ST7735',
      summary: 'ST7735 four-wire commands, address windows and RGB pixels',
      fields: {
        cs: 'Chip-select channel',
        clk: 'Clock channel',
        data: 'Data',
        dc: 'Data/command channel',
      },
      events: {
        begin: 'Begin',
        command: 'Command',
        parameter: 'Parameter',
        window: 'Window',
        pixel: 'Pixel',
        end: 'End',
      },
      eventFields: {
        tx: 'Transaction',
        partial: 'Partial',
        value: 'Value',
        command: 'Command',
        offset: 'Offset',
        column: 'Column',
        first: 'First',
        last: 'Last',
        bits: 'Bits',
        aborted: 'Aborted',
        droppedBits: 'Dropped Bits',
      },
      values: {},
    },
    'zh-CN': {
      name: 'ST7735',
      summary: '解析ST7735四线命令、地址窗口及RGB像素',
      fields: { cs: '片选通道', clk: '时钟通道', data: '数据通道', dc: '数据/命令通道' },
      events: {
        begin: '开始',
        command: '命令',
        parameter: '参数',
        window: '地址窗口',
        pixel: '像素',
        end: '结束',
      },
      eventFields: {
        tx: '事务号',
        partial: '不完整',
        value: '数值',
        command: '命令',
        offset: '偏移',
        column: '列地址',
        first: '起始地址',
        last: '结束地址',
        bits: '位数',
        aborted: '中断',
        droppedBits: '丢弃位数',
      },
      values: {},
    },
  },
  decoder: Display,
})
