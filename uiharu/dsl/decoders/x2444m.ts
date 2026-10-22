// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type -- DSL integer widths and empty configuration records carry IR meaning. */
/** Xicor X2444M eight-bit instruction, RAM reads/writes and STORE/RECALL commands.
 * Manufacturer original: https://bitsavers.trailing-edge.com/components/xicor/1985_Xicor_Data_Book.pdf , X2444/X2444M chapter.
 * CE is active high; input is MSB first. Write-enable/recall latches, EEPROM
 * programming completion and external STORE/RECALL pins are not inferred. */
import type { bool, Provenance, Transaction, u8, u16, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type End = 'complete' | 'incomplete' | 'aborted' | 'extra-data'
export type Event =
  | { kind: 'begin'; tx: u64; opcode: u8; address: u8 }
  | { kind: 'word'; tx: u64; read: bool; value: u16 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('x2444m.events', 1).roles({
  begin: 'command',
  word: 'data',
  end: 'framing',
})
export interface Config {}
class Memory extends BusDecoder<Config, SpiWord, Event, End> {
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
    let bit = (await tx.item()).mosi!
    while ((bit & 1) === 0) bit = (await tx.item()).mosi!
    let header: u8 = 1
    for (let i = 0; i < 7; i++) header = (header << 1) | ((await tx.item()).mosi! & 1)
    const opcode: u8 = header & 7
    const address: u8 = (header >> 3) & 15
    await this.out.emit({ kind: 'begin', tx: tx.id, opcode, address })
    if (opcode === 3 || opcode >= 6) {
      let value: u16 = 0
      const read = opcode >= 6
      for (let i = 0; i < 16; i++) {
        const word = await tx.item()
        value = (value << 1) | ((read ? word.miso! : word.mosi!) & 1)
      }
      await this.out.emit({ kind: 'word', tx: tx.id, read, value })
    }
    let reason: End = 'complete'
    if ((await tx.itemOrEnd()) !== undefined) {
      reason = 'extra-data'
      await tx.drain()
    }
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason })
  }
}
export const memory = defineBusDecoder({
  name: 'x2444m',
  category: 'device',
  config: { defaults: {} },
  input: spiEvents,
  requires: () => ({ wordBits: 1, bitOrder: 'msb-first', cs: true, mosi: true, miso: true }),
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
      name: 'X2444M',
      summary: 'X2444 instruction, address and 16-bit NOVRAM transfers',
      fields: {},
      events: { begin: 'Begin', word: 'Word', end: 'End' },
      eventFields: {
        tx: 'Transaction',
        opcode: 'Opcode',
        address: 'Address',
        read: 'Read',
        value: 'Value',
        reason: 'Reason',
      },
      values: {},
    },
    'zh-CN': {
      name: 'X2444M',
      summary: '解析X2444指令、地址及16位NOVRAM传输',
      fields: {},
      events: { begin: '开始', word: '数据字', end: '结束' },
      eventFields: {
        tx: '事务号',
        opcode: '操作码',
        address: '地址',
        read: '读取',
        value: '数值',
        reason: '结束原因',
      },
      values: {},
    },
  },
  decoder: Memory,
})
