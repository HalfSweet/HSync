// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Cast selects the unwrapped IR byte width. */
/** Generic rising-edge clocked RGB triplets, distinct from APA102. Continuous
 * alignment starts at the first complete byte; a gap suspends alignment until
 * an observed clean CS assertion. CS-less captures therefore require a fresh
 * capture after a gap. Optional framed mode starts each clean selected frame. */
import type { Provenance, Range, Tick, bool, u8, u32, u64 } from 'uiharu'
import { StreamDecoder, at, defineStreamDecoder, eventSchema, join, oneOf } from 'uiharu'
import { spiEvents, type SpiEvent } from './spi'
export type Order = 'rgb' | 'rbg' | 'grb' | 'gbr' | 'brg' | 'bgr'
export type End = 'complete' | 'incomplete' | 'aborted'
export type Event =
  | { kind: 'pixel'; tx: u64; index: u32; red: u8; green: u8; blue: u8 }
  | { kind: 'end'; tx: u64; pixels: u32; reason: End }
export const events = eventSchema<Event>('rgb-led-spi.events', 1).roles({
  pixel: 'data',
  end: 'framing',
})
export interface Config {
  order: Order
  framed: bool
}
class Rgb extends StreamDecoder<Config, SpiEvent, Event> {
  tx: u64 = 1
  pixels: u32 = 0
  count: u8 = 0
  a: u8 = 0
  b: u8 = 0
  aligned = true
  first: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    return this.first?.span.start
  }
  async finish(where: Provenance, reason: End): Promise<void> {
    if (this.count > 0 || this.pixels > 0)
      await this.out.emit({ kind: 'end', tx: this.tx, pixels: this.pixels, reason }, where)
    this.count = 0
    this.pixels = 0
    this.first = undefined
    this.tx += 1
  }
  override async onGap(range: Range): Promise<void> {
    await this.finish(at(range.start), 'aborted')
    this.aligned = false
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.finish(at(tick), this.count > 0 ? 'incomplete' : 'complete')
  }
  override async run(): Promise<void> {
    for (;;) {
      const received = await this.input.next()
      const event = received.event
      const where = received.at
      if (event.kind === 'begin') {
        if (event.partial) {
          await this.finish(where, 'aborted')
          this.aligned = false
          continue
        }
        if (this.config.framed) {
          await this.finish(where, 'aborted')
          this.tx = event.tx
          this.aligned = !event.partial
        } else if (!this.aligned && !event.partial) {
          this.aligned = true
          this.tx = event.tx
        }
      } else if (event.kind === 'end') {
        if (this.config.framed) {
          await this.finish(
            where,
            event.reason === 'aborted' ? 'aborted' : this.count > 0 ? 'incomplete' : 'complete',
          )
          this.aligned = false
        } else if (event.reason === 'aborted') {
          await this.finish(where, this.count > 0 ? 'incomplete' : 'complete')
          this.aligned = false
        }
      } else if (event.kind === 'word') {
        if (!this.aligned) continue
        const value = event.mosi
        if (value === undefined) {
          await this.finish(where, 'aborted')
          this.aligned = false
          continue
        }
        if (this.count === 0) {
          this.a = value
          this.first = where
          this.count = 1
          continue
        }
        if (this.count === 1) {
          this.b = value
          this.count = 2
          continue
        }
        const first = this.first
        if (first === undefined) {
          this.count = 0
          continue
        }
        let red = this.a
        let green = this.b
        let blue: u8 = value as u8
        if (this.config.order === 'rbg') {
          green = value
          blue = this.b
        } else if (this.config.order === 'grb') {
          red = this.b
          green = this.a
        } else if (this.config.order === 'gbr') {
          red = value
          green = this.a
          blue = this.b
        } else if (this.config.order === 'brg') {
          red = this.b
          green = value
          blue = this.a
        } else if (this.config.order === 'bgr') {
          red = value
          green = this.b
          blue = this.a
        }
        this.first = undefined
        this.count = 0
        await this.out.emit(
          { kind: 'pixel', tx: this.tx, index: this.pixels, red, green, blue },
          join(first, where),
        )
        this.pixels += 1
      }
    }
  }
}
export const rgb = defineStreamDecoder({
  name: 'rgb-led-spi',
  category: 'device',
  config: { defaults: { order: 'rgb', framed: false } },
  input: spiEvents,
  requires: (config) => ({
    wordBits: 8,
    bitOrder: 'msb-first',
    mosi: true,
    mode: oneOf(0, 3),
    cs: oneOf(true, config.framed),
  }),
  output: events,
  props: (config) => ({ order: config.order, framed: config.framed }),
  locales: {
    'en-US': {
      name: 'RGB-LED-SPI',
      summary: 'Clocked RGB byte triplets with explicit channel order',
      fields: { order: 'Order', framed: 'Restart triplets on chip select' },
      events: { pixel: 'Pixel', end: 'End' },
      eventFields: {
        tx: 'Transaction',
        index: 'Index',
        red: 'Red',
        green: 'Green',
        blue: 'Blue',
        pixels: 'Pixels',
        reason: 'Reason',
      },
      values: { order: { rgb: 'RGB', rbg: 'RBG', grb: 'GRB', gbr: 'GBR', brg: 'BRG', bgr: 'BGR' } },
    },
    'zh-CN': {
      name: 'RGB-LED-SPI',
      summary: '按指定颜色顺序解析时钟驱动RGB字节组',
      fields: { order: '颜色顺序', framed: '片选重新对齐像素' },
      events: { pixel: '像素', end: '结束' },
      eventFields: {
        tx: '事务号',
        index: '序号',
        red: '红色',
        green: '绿色',
        blue: '蓝色',
        pixels: '像素数',
        reason: '结束原因',
      },
      values: { order: { rgb: 'RGB', rbg: 'RBG', grb: 'GRB', gbr: 'GBR', brg: 'BRG', bgr: 'BGR' } },
    },
  },
  decoder: Rgb,
})
