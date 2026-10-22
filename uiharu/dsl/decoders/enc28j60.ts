// SPDX-License-Identifier: Apache-2.0
/** ENC28J60 RCR/WCR/BFS/BFC, buffer memory and soft reset; observed bank selection, named registers and MAC/MII dummy reads.
 * Public manufacturer specification: https://ww1.microchip.com/downloads/en/devicedoc/39662e.pdf .
 * Each transfer is bounded state and streaming data. Partial CS frames are skipped;
 * missing required data closes incomplete/aborted rather than inventing bytes. */
import type { bool, Provenance, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type Operation =
  | 'soft-reset'
  | 'read-buffer'
  | 'write-buffer'
  | 'read-control'
  | 'write-control'
  | 'bit-set'
  | 'bit-clear'
  | 'read-register'
  | 'write-register'
export type Field =
  | 'interrupt-enable'
  | 'interrupt-flags'
  | 'status'
  | 'control-2'
  | 'bank-select'
  | 'receive-enabled'
  | 'transmit-request'
  | 'dma-start'
  | 'tx-reset'
  | 'clock-ready'
  | 'tx-aborted'
  | 'receive-busy'
  | 'late-collision'
export type DummyMode = 'auto' | 'mac-mii' | 'ethernet'
export type Register =
  | 'unknown'
  | 'reserved'
  | 'eie'
  | 'eir'
  | 'estat'
  | 'econ2'
  | 'econ1'
  | 'erdptl'
  | 'erdpth'
  | 'ewrptl'
  | 'ewrpth'
  | 'etxstl'
  | 'etxsth'
  | 'etxndl'
  | 'etxndh'
  | 'erxstl'
  | 'erxsth'
  | 'erxndl'
  | 'erxndh'
  | 'erxrdptl'
  | 'erxrdpth'
  | 'erxwrptl'
  | 'erxwrpth'
  | 'edmastl'
  | 'edmasth'
  | 'edmandl'
  | 'edmandh'
  | 'edmadstl'
  | 'edmadsth'
  | 'edmacsl'
  | 'edmacsh'
  | 'eht0'
  | 'eht1'
  | 'eht2'
  | 'eht3'
  | 'eht4'
  | 'eht5'
  | 'eht6'
  | 'eht7'
  | 'epmm0'
  | 'epmm1'
  | 'epmm2'
  | 'epmm3'
  | 'epmm4'
  | 'epmm5'
  | 'epmm6'
  | 'epmm7'
  | 'epmcsl'
  | 'epmcsh'
  | 'epmol'
  | 'epmoh'
  | 'erxfcon'
  | 'epktcnt'
  | 'macon1'
  | 'macon3'
  | 'macon4'
  | 'mabbipg'
  | 'maipgl'
  | 'maipgh'
  | 'maclcon1'
  | 'maclcon2'
  | 'mamxfll'
  | 'mamxflh'
  | 'micmd'
  | 'miregadr'
  | 'miwrl'
  | 'miwrh'
  | 'mirdl'
  | 'mirdh'
  | 'maadr5'
  | 'maadr6'
  | 'maadr3'
  | 'maadr4'
  | 'maadr1'
  | 'maadr2'
  | 'ebstsd'
  | 'ebstcon'
  | 'ebstcsl'
  | 'ebstcsh'
  | 'mistat'
  | 'erevid'
  | 'ecocon'
  | 'eflocon'
  | 'epausl'
  | 'epaush'
export type End = 'complete' | 'incomplete' | 'aborted' | 'unsupported' | 'unknown-bank'
export type Event =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'command'; tx: u64; opcode: u8; address: u16; read: bool; status: u8 | undefined }
  | { kind: 'data'; tx: u64; offset: u32; address: u16; value: u8; read: bool }
  | { kind: 'operation'; tx: u64; operation: Operation }
  | { kind: 'field'; tx: u64; address: u16; field: Field; value: u32 }
  | { kind: 'register'; tx: u64; address: u16; bank: u8 | undefined; name: Register }
  | { kind: 'bank'; tx: u64; bank: u8; knownMask: u8 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('enc28j60.events', 1).roles({
  begin: 'framing',
  command: 'command',
  data: 'data',
  operation: 'command',
  field: 'data',
  register: 'command',
  bank: 'data',
  end: 'framing',
})
export interface Config {
  dummyMode: DummyMode
}
interface Layout {
  address: u16
  read: bool
  step: u8
  count: u8 | undefined
  dummy: u8
  valid: bool
}
class Device extends BusDecoder<Config, SpiWord, Event, End> {
  open: u64 | undefined = undefined
  bank: u8 = 0
  knownMask: u8 = 0
  override async recover(reason: End, where: Provenance): Promise<void> {
    this.knownMask = 0
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  isMac(address: u16): bool {
    if (this.bank === 2) return true
    return this.bank === 3 && (address <= 5 || address === 10)
  }
  registerName(address: u16): Register {
    if (address === 27) return 'eie'
    if (address === 28) return 'eir'
    if (address === 29) return 'estat'
    if (address === 30) return 'econ2'
    if (address === 31) return 'econ1'
    if (this.knownMask !== 3) return 'unknown'
    if (this.bank === 0) {
      if (address === 0) return 'erdptl'
      if (address === 1) return 'erdpth'
      if (address === 2) return 'ewrptl'
      if (address === 3) return 'ewrpth'
      if (address === 4) return 'etxstl'
      if (address === 5) return 'etxsth'
      if (address === 6) return 'etxndl'
      if (address === 7) return 'etxndh'
      if (address === 8) return 'erxstl'
      if (address === 9) return 'erxsth'
      if (address === 10) return 'erxndl'
      if (address === 11) return 'erxndh'
      if (address === 12) return 'erxrdptl'
      if (address === 13) return 'erxrdpth'
      if (address === 14) return 'erxwrptl'
      if (address === 15) return 'erxwrpth'
      if (address === 16) return 'edmastl'
      if (address === 17) return 'edmasth'
      if (address === 18) return 'edmandl'
      if (address === 19) return 'edmandh'
      if (address === 20) return 'edmadstl'
      if (address === 21) return 'edmadsth'
      if (address === 22) return 'edmacsl'
      if (address === 23) return 'edmacsh'
    }
    if (this.bank === 1) {
      if (address === 0) return 'eht0'
      if (address === 1) return 'eht1'
      if (address === 2) return 'eht2'
      if (address === 3) return 'eht3'
      if (address === 4) return 'eht4'
      if (address === 5) return 'eht5'
      if (address === 6) return 'eht6'
      if (address === 7) return 'eht7'
      if (address === 8) return 'epmm0'
      if (address === 9) return 'epmm1'
      if (address === 10) return 'epmm2'
      if (address === 11) return 'epmm3'
      if (address === 12) return 'epmm4'
      if (address === 13) return 'epmm5'
      if (address === 14) return 'epmm6'
      if (address === 15) return 'epmm7'
      if (address === 16) return 'epmcsl'
      if (address === 17) return 'epmcsh'
      if (address === 20) return 'epmol'
      if (address === 21) return 'epmoh'
      if (address === 24) return 'erxfcon'
      if (address === 25) return 'epktcnt'
    }
    if (this.bank === 2) {
      if (address === 0) return 'macon1'
      if (address === 2) return 'macon3'
      if (address === 3) return 'macon4'
      if (address === 4) return 'mabbipg'
      if (address === 6) return 'maipgl'
      if (address === 7) return 'maipgh'
      if (address === 8) return 'maclcon1'
      if (address === 9) return 'maclcon2'
      if (address === 10) return 'mamxfll'
      if (address === 11) return 'mamxflh'
      if (address === 18) return 'micmd'
      if (address === 20) return 'miregadr'
      if (address === 22) return 'miwrl'
      if (address === 23) return 'miwrh'
      if (address === 24) return 'mirdl'
      if (address === 25) return 'mirdh'
    }
    if (this.bank === 3) {
      if (address === 0) return 'maadr5'
      if (address === 1) return 'maadr6'
      if (address === 2) return 'maadr3'
      if (address === 3) return 'maadr4'
      if (address === 4) return 'maadr1'
      if (address === 5) return 'maadr2'
      if (address === 6) return 'ebstsd'
      if (address === 7) return 'ebstcon'
      if (address === 8) return 'ebstcsl'
      if (address === 9) return 'ebstcsh'
      if (address === 10) return 'mistat'
      if (address === 18) return 'erevid'
      if (address === 21) return 'ecocon'
      if (address === 23) return 'eflocon'
      if (address === 24) return 'epausl'
      if (address === 25) return 'epaush'
    }
    return 'reserved'
  }
  async observeBank(
    tx: u64,
    opcode: u8,
    address: u16,
    value: u8,
    where: Provenance,
  ): Promise<void> {
    if (address !== 31) return
    const command: u8 = opcode & 0xe0
    if (command === 0 || command === 0x40) {
      this.bank = value & 3
      this.knownMask = 3
    }
    if (command === 0x80) {
      this.bank |= value & 3
      this.knownMask |= value & 3
    }
    if (command === 0xa0) {
      this.bank &= ~value & 3
      this.knownMask |= value & 3
    }
    await this.out.emit({ kind: 'bank', tx, bank: this.bank, knownMask: this.knownMask }, where)
  }
  layout(opcode: u8): Layout {
    const address: u16 = opcode & 31
    const command: u8 = opcode & 0xe0
    if (opcode === 0xff)
      return { address: 0, read: false, step: 0, count: 0, dummy: 0, valid: true }
    if (opcode === 0x3a || opcode === 0x7a)
      return { address: 0, read: opcode === 0x3a, step: 0, count: undefined, dummy: 0, valid: true }
    return {
      address,
      read: command === 0,
      step: 0,
      count: 1,
      dummy:
        command === 0 &&
        address < 27 &&
        (this.config.dummyMode === 'mac-mii' ||
          (this.config.dummyMode === 'auto' && this.knownMask === 3 && this.isMac(address)))
          ? 1
          : 0,
      valid: command === 0 || command === 0x40 || command === 0x80 || command === 0xa0,
    }
  }
  operation(opcode: u8, read: bool): Operation {
    if (opcode === 0xff) return 'soft-reset'
    if (opcode === 0x3a) return 'read-buffer'
    if (opcode === 0x7a) return 'write-buffer'
    if ((opcode & 0xe0) === 0) return 'read-control'
    if ((opcode & 0xe0) === 0x40) return 'write-control'
    if ((opcode & 0xe0) === 0x80) return 'bit-set'
    if ((opcode & 0xe0) === 0xa0) return 'bit-clear'
    return read ? 'read-register' : 'write-register'
  }
  async interpret(tx: u64, address: u16, value: u8, where: Provenance): Promise<void> {
    if (address === 27)
      await this.out.emit({ kind: 'field', tx, address, field: 'interrupt-enable', value }, where)
    if (address === 28)
      await this.out.emit({ kind: 'field', tx, address, field: 'interrupt-flags', value }, where)
    if (address === 29)
      await this.out.emit({ kind: 'field', tx, address, field: 'status', value }, where)
    if (address === 30)
      await this.out.emit({ kind: 'field', tx, address, field: 'control-2', value }, where)
    if (address === 31) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'bank-select', value: value & 3 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'receive-enabled', value: (value >> 2) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'transmit-request', value: (value >> 3) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'dma-start', value: (value >> 5) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-reset', value: value >> 7 },
        where,
      )
    }
    if (address === 29) {
      await this.out.emit(
        { kind: 'field', tx, address, field: 'clock-ready', value: value & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'tx-aborted', value: (value >> 1) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'receive-busy', value: (value >> 2) & 1 },
        where,
      )
      await this.out.emit(
        { kind: 'field', tx, address, field: 'late-collision', value: (value >> 4) & 1 },
        where,
      )
    }
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    this.open = tx.id
    await this.out.emit({ kind: 'begin', tx: tx.id })
    let offset: u32 = 0
    for (;;) {
      const first = await tx.itemOrEnd()
      if (first === undefined) break
      const opcode = first.mosi!
      const layout = this.layout(opcode)
      const commandAt = this.consumed()
      await this.out.emit(
        {
          kind: 'command',
          tx: tx.id,
          opcode,
          address: layout.address,
          read: layout.read,
          status: first.miso,
        },
        commandAt,
      )
      await this.out.emit(
        { kind: 'operation', tx: tx.id, operation: this.operation(opcode, layout.read) },
        commandAt,
      )
      if (!layout.valid) {
        await tx.drain()
        this.open = undefined
        await this.out.emit({ kind: 'end', tx: tx.id, reason: 'unsupported' })
        return
      }
      if (opcode === 0xff) {
        this.bank = 0
        this.knownMask = 3
        await this.out.emit({ kind: 'bank', tx: tx.id, bank: 0, knownMask: 3 }, commandAt)
      }
      if (opcode !== 0xff && opcode !== 0x3a && opcode !== 0x7a) {
        let observedBank: u8 | undefined = undefined
        if (this.knownMask === 3) observedBank = this.bank
        await this.out.emit(
          {
            kind: 'register',
            tx: tx.id,
            address: layout.address,
            bank: observedBank,
            name: this.registerName(layout.address),
          },
          commandAt,
        )
        if (
          layout.read &&
          layout.address < 27 &&
          this.config.dummyMode === 'auto' &&
          this.knownMask !== 3
        ) {
          await tx.drain()
          this.open = undefined
          await this.out.emit({ kind: 'end', tx: tx.id, reason: 'unknown-bank' })
          return
        }
      }
      for (let i = 0; i < layout.dummy; i++) await tx.item()
      let address = layout.address
      let count: u32 = 0
      for (;;) {
        if (layout.count !== undefined && count >= layout.count) break
        const word = layout.count === undefined ? await tx.itemOrEnd() : await tx.item()
        if (word === undefined) {
          this.open = undefined
          await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
          return
        }
        const value = layout.read ? word.miso! : word.mosi!
        const where = this.consumed()
        await this.out.emit(
          { kind: 'data', tx: tx.id, offset, address, value, read: layout.read },
          where,
        )
        if (opcode !== 0x3a && opcode !== 0x7a) {
          await this.interpret(tx.id, address, value, where)
          await this.observeBank(tx.id, opcode, address, value, where)
        }
        count += 1
        offset += 1
        address += layout.step
      }
    }
    this.open = undefined
    await this.out.emit({ kind: 'end', tx: tx.id, reason: 'complete' })
  }
}
export const device = defineBusDecoder({
  name: 'enc28j60',
  category: 'device',
  config: { defaults: { dummyMode: 'auto' } },
  input: spiEvents,
  requires: () => ({ wordBits: 8, bitOrder: 'msb-first', cs: true, mosi: true, miso: true }),
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
      name: 'ENC28J60',
      summary: 'ENC28J60 bank tracking, named registers, automatic MAC/MII dummy reads and buffers',
      fields: { dummyMode: 'Register read dummy handling' },
      events: {
        begin: 'Begin',
        command: 'Command',
        data: 'Data',
        operation: 'Operation',
        field: 'Field',
        end: 'End',
        register: 'Register',
        bank: 'Bank',
      },
      eventFields: {
        tx: 'Transaction',
        opcode: 'Opcode',
        address: 'Address',
        read: 'Read',
        status: 'Status',
        offset: 'Offset',
        value: 'Value',
        operation: 'Operation',
        field: 'Field',
        reason: 'Reason',
        bank: 'Bank',
        knownMask: 'Known bank-bit mask',
        name: 'Register name',
      },
      values: {
        dummyMode: {
          auto: 'Follow observed bank',
          'mac-mii': 'MAC/MII (one dummy byte)',
          ethernet: 'Ethernet (no dummy byte)',
        },
      },
    },
    'zh-CN': {
      name: 'ENC28J60',
      summary: '解析ENC28J60寄存器分组、命名寄存器、自动MAC/MII空字节及缓冲区操作',
      fields: { dummyMode: '寄存器读取空字节模式' },
      events: {
        begin: '开始',
        command: '命令',
        data: '数据通道',
        operation: '操作',
        field: '字段',
        end: '结束',
        register: '寄存器',
        bank: '寄存器分组',
      },
      eventFields: {
        tx: '事务号',
        opcode: '操作码',
        address: '地址',
        read: '读取',
        status: '状态',
        offset: '偏移',
        value: '数值',
        operation: '操作',
        field: '字段',
        reason: '结束原因',
        bank: '寄存器分组',
        knownMask: '已知分组位掩码',
        name: '寄存器名称',
      },
      values: {
        dummyMode: {
          auto: '自动跟随已知分组',
          'mac-mii': 'MAC/MII（一个空字节）',
          ethernet: '以太网（无空字节）',
        },
      },
    },
  },
  decoder: Device,
})
