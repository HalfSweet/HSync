// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type, @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */

/** AD5626 twelve-bit serial input code; LDAC is not observed.
 * Public specification: https://www.analog.com/media/en/technical-documentation/data-sheets/ad5626.pdf .
 * Exactly 12 clocks per selected frame are required. Partial frames are drained;
 * short, overlong and truncated frames never emit a completed semantic value. */
import type { Provenance, Transaction, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type End = 'complete' | 'incomplete' | 'aborted' | 'invalid-length'
export type DeviceEvent =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'dac'; tx: u64; code: u16; microvolts: u32 }
  | { kind: 'end'; tx: u64; reason: End }
export const deviceEvents = eventSchema<DeviceEvent>('ad5626.events', 1).roles({
  begin: 'framing',
  dac: 'data',
  end: 'framing',
})
export interface Config {}
class Device extends BusDecoder<Config, SpiWord, DeviceEvent, End> {
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
    let value: u32 = 0
    for (let i = 0; i < 12; i++) value = (value << 1) | ((await tx.item()).mosi! & 1)
    if ((await tx.itemOrEnd()) !== undefined) {
      await tx.drain()
      this.open = undefined
      await this.out.emit({ kind: 'end', tx: tx.id, reason: 'invalid-length' })
      return
    }
    await this.out.emit({
      kind: 'dac',
      tx: tx.id,
      code: (value & 0xfff) as u16,
      microvolts: (value & 0xfff) * 1000,
    })
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
  }
}
export const device = defineBusDecoder({
  name: 'ad5626',
  category: 'device',
  config: { defaults: {} },
  input: spiEvents,
  requires: () => ({ wordBits: 1, bitOrder: 'msb-first', cs: true, mosi: true }),
  output: deviceEvents,
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
      name: 'AD5626',
      summary: 'AD5626 12-bit DAC codes and nominal output microvolts',
      fields: {},
      events: { begin: 'Begin', dac: 'Dac', end: 'End' },
      eventFields: {
        tx: 'Transaction',
        code: 'Code',
        microvolts: 'Voltage (µV)',
        reason: 'Reason',
      },
      values: {},
    },
    'zh-CN': {
      name: 'AD5626',
      summary: '解析AD5626的12位DAC编码及标称输出微伏值',
      fields: {},
      events: { begin: '开始', dac: 'DAC输出', end: '结束' },
      eventFields: { tx: '事务号', code: '编码', microvolts: '电压（微伏）', reason: '结束原因' },
      values: {},
    },
  },
  decoder: Device,
})
