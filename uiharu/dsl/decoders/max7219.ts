// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type -- DSL integer widths and empty configuration records carry IR meaning. */
/** MAX7219/MAX7221 serial register words and daisy-chain slots:
 * https://www.analog.com/media/en/technical-documentation/data-sheets/MAX7219-MAX7221.pdf .
 * Each 16-bit shifted word contains address bits11:8 and eight data bits.
 * Slots are wire order, not physical chip number. A word reports shifted content;
 * only a complete end confirms LOAD/CS release. No display image is inferred. */
import type { bool, Provenance, Transaction, u8, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type End = 'complete' | 'incomplete' | 'aborted'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'register'; tx: u64; slot: u32; address: u8; value: u8 }
  | { kind: 'digit'; tx: u64; slot: u32; digit: u8; segments: u8; decimalPoint: bool }
  | { kind: 'decode-mode'; tx: u64; slot: u32; mask: u8 }
  | { kind: 'intensity'; tx: u64; slot: u32; level: u8 }
  | { kind: 'scan-limit'; tx: u64; slot: u32; lastDigit: u8 }
  | { kind: 'shutdown'; tx: u64; slot: u32; enabled: bool }
  | { kind: 'display-test'; tx: u64; slot: u32; enabled: bool }
  | { kind: 'warning'; tx: u64; slot: u32; address: u8; value: u8 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('max7219.events', 1).roles({
  begin: 'framing',
  register: 'data',
  digit: 'data',
  'decode-mode': 'control',
  intensity: 'control',
  'scan-limit': 'control',
  shutdown: 'control',
  'display-test': 'control',
  warning: 'control',
  end: 'framing',
})
export interface Config {}
class Device extends BusDecoder<Config, SpiWord, Event, End> {
  open: u64 | undefined = undefined
  override async recover(reason: End, where: Provenance): Promise<void> {
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    this.open = tx.id
    await this.out.emit({ kind: 'begin', tx: tx.id })
    let slot: u32 = 0
    for (;;) {
      const high = await tx.itemOrEnd()
      if (high === undefined) break
      const address: u8 = high.mosi! & 15
      const value = (await tx.item()).mosi!
      const where = this.consumed()
      await this.out.emit({ kind: 'register', tx: tx.id, slot, address, value }, where)
      if (address >= 1 && address <= 8)
        await this.out.emit(
          {
            kind: 'digit',
            tx: tx.id,
            slot,
            digit: address - 1,
            segments: value & 127,
            decimalPoint: (value & 128) !== 0,
          },
          where,
        )
      else if (address === 9)
        await this.out.emit({ kind: 'decode-mode', tx: tx.id, slot, mask: value }, where)
      else if (address === 10) {
        await this.out.emit({ kind: 'intensity', tx: tx.id, slot, level: value & 15 }, where)
        if (value > 15)
          await this.out.emit({ kind: 'warning', tx: tx.id, slot, address, value }, where)
      } else if (address === 11) {
        await this.out.emit({ kind: 'scan-limit', tx: tx.id, slot, lastDigit: value & 7 }, where)
        if (value > 7)
          await this.out.emit({ kind: 'warning', tx: tx.id, slot, address, value }, where)
      } else if (address === 12)
        await this.out.emit(
          { kind: 'shutdown', tx: tx.id, slot, enabled: (value & 1) === 0 },
          where,
        )
      else if (address === 15)
        await this.out.emit(
          { kind: 'display-test', tx: tx.id, slot, enabled: (value & 1) !== 0 },
          where,
        )
      else if (address !== 0)
        await this.out.emit({ kind: 'warning', tx: tx.id, slot, address, value }, where)
      slot += 1
    }
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
  }
}
export const device = defineBusDecoder({
  name: 'max7219',
  category: 'device',
  config: { defaults: {} },
  input: spiEvents,
  requires: () => ({ wordBits: 8, bitOrder: 'msb-first', cs: true, mosi: true }),
  output: events,
  props: () => ({}),
  partial: 'drain',
  recovery: {
    incomplete: 'incomplete',
    truncated: 'aborted',
    missing: 'aborted',
    gap: 'aborted',
    end: 'aborted',
  },

  locales: {
    'en-US': {
      name: 'MAX7219',
      summary: 'MAX7219/MAX7221 digits, display controls and reserved-register warnings',
      fields: {},
      events: {
        begin: 'Begin',
        register: 'Register',
        digit: 'Digit',
        'decode-mode': 'Decode mode',
        intensity: 'Intensity',
        'scan-limit': 'Scan limit',
        shutdown: 'Shutdown',
        'display-test': 'Display test',
        warning: 'Warning',
        end: 'End',
      },
      eventFields: {
        tx: 'Transaction',
        slot: 'Slot',
        address: 'Address',
        value: 'Value',
        digit: 'Digit',
        segments: 'Segments',
        decimalPoint: 'Decimal Point',
        mask: 'Mask',
        level: 'Level',
        lastDigit: 'Last Digit',
        enabled: 'Enabled',
        reason: 'Reason',
      },
      values: {},
    },
    'zh-CN': {
      name: 'MAX7219',
      summary: '解析MAX7219/MAX7221数位、显示控制及保留寄存器警告',
      fields: {},
      events: {
        begin: '开始',
        register: '寄存器',
        digit: '数位',
        'decode-mode': '译码模式',
        intensity: '亮度',
        'scan-limit': '扫描位数',
        shutdown: '关断',
        'display-test': '显示测试',
        warning: '警告',
        end: '结束',
      },
      eventFields: {
        tx: '事务号',
        slot: '链中序号',
        address: '地址',
        value: '数值',
        digit: '数位',
        segments: '段码',
        decimalPoint: '小数点',
        mask: '掩码',
        level: '等级',
        lastDigit: '末尾数位',
        enabled: '启用',
        reason: '结束原因',
      },
      values: {},
    },
  },
  decoder: Device,
})
