// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-empty-object-type, @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */
/** SD SPI CID/CSD/OCR register interpretation, SD Physical Layer Simplified Spec
 * §§5.1–5.3/7.3: https://www.sdcard.org/downloads/pls/ .
 * Does not replace sdcard-spi command/block decoder. CMD9/10/58 only; CID fields,
 * CSD v1/v2 block capacity and OCR flags. No UHS/SDUC/CSD-v3 interpretation. */
import type { bool, Provenance, Tick, Transaction, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, join } from 'uiharu'
import { spiEvents, type SpiWord } from './spi'
export type End = 'complete' | 'incomplete' | 'aborted' | 'unsupported' | 'rejected'
export type Event =
  | {
      kind: 'scr'
      tx: u64
      raw: u64
      structure: u8
      sdSpec: u8
      dataAfterErase: bool
      security: u8
      busWidths: u8
      spec3: bool
      extendedSecurity: u8
      spec4: bool
      specExtension: u8
      commandSupport: u8
      vendorData: u32
      crcOk: bool
      supported: bool
    }
  | { kind: 'begin'; tx: u64; command: u8 }
  | {
      kind: 'cid'
      tx: u64
      productName: [u8, u8, u8, u8, u8]
      manufacturer: u8
      oem: u16
      revision: u8
      serial: u32
      year: u16
      month: u8
      name0: u8
      name1: u8
      name2: u8
      name3: u8
      name4: u8
      registerCrcOk: bool
      registerEndOk: bool
      crcOk: bool
    }
  | {
      kind: 'csd'
      tx: u64
      taac: u8
      nsac: u8
      transferSpeed: u8
      commandClasses: u16
      readPartial: bool
      writeMisalign: bool
      readMisalign: bool
      dsrImplemented: bool
      readCurrentMin: u8 | undefined
      readCurrentMax: u8 | undefined
      writeCurrentMin: u8 | undefined
      writeCurrentMax: u8 | undefined
      sizeMultiplier: u8 | undefined
      eraseSingleBlock: bool
      eraseSectorBlocks: u8
      writeProtectGroupSectors: u8 | undefined
      writeProtectGroupEnabled: bool | undefined
      writeSpeedFactor: u8
      writeBlockBits: u8
      writePartial: bool
      fileFormatGroup: bool
      copied: bool
      permanentWriteProtect: bool
      temporaryWriteProtect: bool
      fileFormat: u8
      registerCrc: u8
      structure: u8
      readBlockBits: u8
      size: u32
      blocks: u64
      registerCrcOk: bool
      registerEndOk: bool
      crcOk: bool
      supported: bool
    }
  | { kind: 'ocr'; tx: u64; value: u32; ready: bool; highCapacity: bool; switch18: bool }
  | { kind: 'status'; tx: u64; value: u8 }
  | { kind: 'end'; tx: u64; reason: End }
export const events = eventSchema<Event>('sd-registers.events', 1).roles({
  scr: { role: 'data', errorWhen: { crcOk: false } },
  begin: 'command',
  cid: 'data',
  csd: 'data',
  ocr: 'data',
  status: 'data',
  end: 'framing',
})
export interface Config {}
function crcByte(crc: u16, value: u8): u16 {
  let result: u16 = crc ^ (value << 8)
  for (let i = 0; i < 8; i++)
    result = (result & 0x8000) !== 0 ? (result << 1) ^ 0x1021 : result << 1
  return result
}
function crc7Byte(crc: u8, value: u8): u8 {
  let result: u8 = crc
  for (let bit: u8 = 0; bit < 8; bit += 1) {
    const feedback: u8 = (result >> 6) ^ ((value >> (7 - bit)) & 1)
    result = (result << 1) & 127
    if ((feedback & 1) !== 0) result ^= 9
  }
  return result
}
class Registers extends BusDecoder<Config, SpiWord, Event, End> {
  open: u64 | undefined = undefined
  appAccepted = false
  scrAt: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    return this.scrAt?.span.start
  }
  override async recover(reason: End, where: Provenance): Promise<void> {
    this.appAccepted = false
    this.scrAt = undefined
    if (this.open !== undefined) {
      const tx = this.open
      this.open = undefined
      await this.out.emit({ kind: 'end', tx, reason }, where)
    }
  }
  async finish(tx: u64, reason: End): Promise<void> {
    this.open = undefined
    await this.out.emit({ kind: 'end', tx, reason })
  }
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    let command = (await tx.item()).mosi!
    while (command === 0xff) command = (await tx.item()).mosi!
    // SD command tokens start with 0 (start) and 1 (host transmission).
    // Do not reinterpret arbitrary SPI bytes as their low six command bits.
    if ((command & 0xc0) !== 0x40) {
      this.appAccepted = false
      await tx.drain()
      return
    }
    command &= 63
    const application = this.appAccepted
    this.appAccepted = false
    if (
      command !== 9 &&
      command !== 10 &&
      command !== 58 &&
      command !== 55 &&
      !(command === 51 && application)
    ) {
      await tx.drain()
      return
    }
    this.open = tx.id
    await this.out.emit({ kind: 'begin', tx: tx.id, command })
    for (let i = 0; i < 5; i++) await tx.item()
    let status = (await tx.item()).miso!
    while ((status & 128) !== 0) status = (await tx.item()).miso!
    await this.out.emit({ kind: 'status', tx: tx.id, value: status })
    if (command === 55) {
      this.appAccepted = status <= 1
      await tx.drain()
      await this.finish(tx.id, status <= 1 ? 'complete' : 'rejected')
      return
    }
    if (status !== 0) {
      await tx.drain()
      await this.finish(tx.id, 'rejected')
      return
    }
    if (command === 58) {
      let value: u32 = 0
      for (let i = 0; i < 4; i++) value = (value << 8) | (await tx.item()).miso!
      await this.out.emit({
        kind: 'ocr',
        tx: tx.id,
        value,
        ready: (value & 0x80000000) !== 0,
        highCapacity: (value & 0x40000000) !== 0,
        switch18: (value & 0x1000000) !== 0,
      })
      await tx.drain()
      await this.finish(tx.id, 'complete')
      return
    }
    let token = (await tx.item()).miso!
    while (token === 0xff) token = (await tx.item()).miso!
    if (token !== 0xfe) {
      await this.out.emit({ kind: 'status', tx: tx.id, value: token })
      await tx.drain()
      await this.finish(tx.id, 'rejected')
      return
    }
    if (command === 51) {
      let a: u32 = 0
      let b: u32 = 0
      let crc: u16 = 0
      for (let i = 0; i < 8; i++) {
        const value = (await tx.item()).miso!
        const where = this.consumed()
        this.scrAt = this.scrAt === undefined ? where : join(this.scrAt, where)
        crc = crcByte(crc, value)
        if (i < 4) a = (a << 8) | value
        else b = (b << 8) | value
      }
      const high = (await tx.item()).miso!
      const low = (await tx.item()).miso!
      const first = this.scrAt
      const last = this.consumed()
      this.scrAt = undefined
      if (first !== undefined) {
        await this.out.emit(
          {
            kind: 'scr',
            tx: tx.id,
            raw: ((a as u64) << 32) | (b as u64),
            structure: (a >> 28) as u8,
            sdSpec: ((a >> 24) & 15) as u8,
            dataAfterErase: ((a >> 23) & 1) !== 0,
            security: ((a >> 20) & 7) as u8,
            busWidths: ((a >> 16) & 15) as u8,
            spec3: ((a >> 15) & 1) !== 0,
            extendedSecurity: ((a >> 11) & 15) as u8,
            spec4: ((a >> 10) & 1) !== 0,
            specExtension: ((a >> 6) & 15) as u8,
            commandSupport: (a & 15) as u8,
            vendorData: b,
            crcOk: crc === ((high << 8) | low),
            supported: a >> 28 === 0,
          },
          join(first, last),
        )
      }
      await tx.drain()
      await this.finish(tx.id, 'complete')
      return
    }
    let a: u32 = 0
    let b: u32 = 0
    let c: u32 = 0
    let d: u32 = 0
    let crc: u16 = 0
    let registerCrc: u8 = 0
    for (let i = 0; i < 16; i++) {
      const value = (await tx.item()).miso!
      crc = crcByte(crc, value)
      if (i < 15) registerCrc = crc7Byte(registerCrc, value)
      if (i < 4) a = (a << 8) | value
      else if (i < 8) b = (b << 8) | value
      else if (i < 12) c = (c << 8) | value
      else d = (d << 8) | value
    }
    const high = (await tx.item()).miso!
    const low = (await tx.item()).miso!
    const crcOk = crc === ((high << 8) | low)
    if (command === 10) {
      const date: u16 = ((d >> 8) & 0xfff) as u16
      await this.out.emit({
        kind: 'cid',
        tx: tx.id,
        productName: [
          (a & 255) as u8,
          (b >> 24) as u8,
          ((b >> 16) & 255) as u8,
          ((b >> 8) & 255) as u8,
          (b & 255) as u8,
        ],
        manufacturer: (a >> 24) as u8,
        oem: ((a >> 8) & 0xffff) as u16,
        revision: (c >> 24) as u8,
        serial: ((c & 0xffffff) << 8) | (d >> 24),
        year: 2000 + (date >> 4),
        month: (date & 15) as u8,
        name0: (a & 255) as u8,
        name1: (b >> 24) as u8,
        name2: ((b >> 16) & 255) as u8,
        name3: ((b >> 8) & 255) as u8,
        name4: (b & 255) as u8,
        registerCrcOk: registerCrc === ((d >> 1) & 127),
        registerEndOk: (d & 1) !== 0,
        crcOk,
      })
    } else {
      const structure: u8 = (a >> 30) as u8
      const readBlockBits: u8 = ((b >> 16) & 15) as u8
      let size: u32 = 0
      let blocks: u64 = 0
      if (structure === 1) {
        size = ((b & 63) << 16) | (c >> 16)
        blocks = (size + 1) * 1024
      } else if (structure === 0) {
        size = ((b & 0x3ff) << 2) | (c >> 30)
        const multiplier: u8 = ((c >> 15) & 7) as u8
        blocks = ((size + 1) * (1 << (multiplier + 2)) * (1 << readBlockBits)) / 512
      }
      await this.out.emit({
        kind: 'csd',
        tx: tx.id,
        taac: (a >> 16) as u8,
        nsac: ((a >> 8) & 255) as u8,
        transferSpeed: (a & 255) as u8,
        commandClasses: (b >> 20) as u16,
        readPartial: (b & 0x8000) !== 0,
        writeMisalign: (b & 0x4000) !== 0,
        readMisalign: (b & 0x2000) !== 0,
        dsrImplemented: (b & 0x1000) !== 0,
        readCurrentMin: structure === 0 ? (((c >> 27) & 7) as u8) : undefined,
        readCurrentMax: structure === 0 ? (((c >> 24) & 7) as u8) : undefined,
        writeCurrentMin: structure === 0 ? (((c >> 21) & 7) as u8) : undefined,
        writeCurrentMax: structure === 0 ? (((c >> 18) & 7) as u8) : undefined,
        sizeMultiplier: structure === 0 ? (((c >> 15) & 7) as u8) : undefined,
        eraseSingleBlock: (c & 0x4000) !== 0,
        eraseSectorBlocks: (((c >> 7) & 127) + 1) as u8,
        writeProtectGroupSectors: structure === 0 ? (((c & 127) + 1) as u8) : undefined,
        writeProtectGroupEnabled: structure === 0 ? (d & 0x80000000) !== 0 : undefined,
        writeSpeedFactor: ((d >> 26) & 7) as u8,
        writeBlockBits: ((d >> 22) & 15) as u8,
        writePartial: (d & 0x200000) !== 0,
        fileFormatGroup: (d & 0x8000) !== 0,
        copied: (d & 0x4000) !== 0,
        permanentWriteProtect: (d & 0x2000) !== 0,
        temporaryWriteProtect: (d & 0x1000) !== 0,
        fileFormat: ((d >> 10) & 3) as u8,
        registerCrc: ((d >> 1) & 127) as u8,
        structure,
        readBlockBits,
        size,
        blocks,
        registerCrcOk: registerCrc === ((d >> 1) & 127),
        registerEndOk: (d & 1) !== 0,
        crcOk,
        supported: structure <= 1,
      })
    }
    await tx.drain()
    await this.finish(tx.id, 'complete')
  }
}
export const registers = defineBusDecoder({
  name: 'sd-registers',
  category: 'device',
  config: { defaults: {} },
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
      name: 'SD-REGISTERS',
      summary: 'SD SPI CID, CSD v1/v2 capacity and OCR fields',
      fields: {},
      events: {
        begin: 'Begin',
        cid: 'Cid',
        csd: 'Csd',
        ocr: 'Ocr',
        status: 'Status',
        end: 'End',
        scr: 'SCR register',
      },
      eventFields: {
        registerCrcOk: 'Register CRC7 valid',
        registerEndOk: 'Register end bit valid',
        taac: 'TAAC access-time code',
        nsac: 'NSAC clock-count code',
        transferSpeed: 'Maximum transfer-rate code',
        commandClasses: 'Command class mask',
        readPartial: 'Partial read blocks',
        writeMisalign: 'Write block misalignment',
        readMisalign: 'Read block misalignment',
        dsrImplemented: 'DSR implemented',
        readCurrentMin: 'Minimum-voltage read-current code',
        readCurrentMax: 'Maximum-voltage read-current code',
        writeCurrentMin: 'Minimum-voltage write-current code',
        writeCurrentMax: 'Maximum-voltage write-current code',
        sizeMultiplier: 'Capacity multiplier code',
        eraseSingleBlock: 'Single-block erase',
        eraseSectorBlocks: 'Erase sector write blocks',
        writeProtectGroupSectors: 'Write-protect group sectors',
        writeProtectGroupEnabled: 'Group write protection',
        writeSpeedFactor: 'Write/read time factor code',
        writeBlockBits: 'Write block length exponent',
        writePartial: 'Partial write blocks',
        fileFormatGroup: 'File format group',
        copied: 'Copy flag',
        permanentWriteProtect: 'Permanent write protection',
        temporaryWriteProtect: 'Temporary write protection',
        fileFormat: 'File format code',
        registerCrc: 'Register CRC7',
        productName: 'Product name bytes',
        tx: 'Transaction',
        command: 'Command',
        manufacturer: 'Manufacturer',
        oem: 'OEM',
        revision: 'Revision',
        serial: 'Serial',
        year: 'Year',
        month: 'Month',
        name0: 'Name0',
        name1: 'Name1',
        name2: 'Name2',
        name3: 'Name3',
        name4: 'Name4',
        crcOk: 'CRC valid',
        structure: 'Structure',
        readBlockBits: 'Read Block Bits',
        size: 'Size',
        blocks: 'Blocks',
        supported: 'Supported',
        value: 'Value',
        ready: 'Ready',
        highCapacity: 'High Capacity',
        switch18: 'Switch18',
        reason: 'Reason',
        raw: 'Raw SCR',
        sdSpec: 'SD specification code',
        dataAfterErase: 'Erased data is one',
        security: 'Security version',
        busWidths: 'Supported bus widths',
        spec3: 'Version 3 or later',
        extendedSecurity: 'Extended security',
        spec4: 'Version 4 or later',
        specExtension: 'Specification extension',
        commandSupport: 'Supported commands',
        vendorData: 'Manufacturer data',
      },
      values: {},
    },
    'zh-CN': {
      name: 'SD-REGISTERS',
      summary: '解析SD卡SPI模式CID、CSD v1/v2容量及OCR字段',
      fields: {},
      events: {
        begin: '开始',
        cid: 'CID标识',
        csd: 'CSD配置',
        ocr: 'OCR操作条件',
        status: '状态',
        end: '结束',
        scr: 'SCR配置寄存器',
      },
      eventFields: {
        registerCrcOk: '寄存器CRC7有效',
        registerEndOk: '寄存器结束位有效',
        taac: 'TAAC访问时间编码',
        nsac: 'NSAC时钟计数编码',
        transferSpeed: '最大传输速率编码',
        commandClasses: '命令类别掩码',
        readPartial: '支持部分读块',
        writeMisalign: '允许写块地址未对齐',
        readMisalign: '允许读块地址未对齐',
        dsrImplemented: '实现DSR寄存器',
        readCurrentMin: '最低电压读取电流编码',
        readCurrentMax: '最高电压读取电流编码',
        writeCurrentMin: '最低电压写入电流编码',
        writeCurrentMax: '最高电压写入电流编码',
        sizeMultiplier: '容量倍数编码',
        eraseSingleBlock: '支持单块擦除',
        eraseSectorBlocks: '每擦除扇区的写块数',
        writeProtectGroupSectors: '每写保护组的扇区数',
        writeProtectGroupEnabled: '支持组写保护',
        writeSpeedFactor: '写入与读取时间倍数编码',
        writeBlockBits: '写块长度指数',
        writePartial: '支持部分写块',
        fileFormatGroup: '文件格式组',
        copied: '副本标志',
        permanentWriteProtect: '永久写保护',
        temporaryWriteProtect: '临时写保护',
        fileFormat: '文件格式编码',
        registerCrc: '寄存器CRC7',
        productName: '产品名字节',
        tx: '事务号',
        command: '命令',
        manufacturer: '制造商',
        oem: 'OEM',
        revision: '版本',
        serial: '序列号',
        year: '年份',
        month: '月份',
        name0: '产品名第1字节',
        name1: '产品名第2字节',
        name2: '产品名第3字节',
        name3: '产品名第4字节',
        name4: '产品名第5字节',
        crcOk: 'CRC有效',
        structure: '结构版本',
        readBlockBits: '读块位数',
        size: '容量字段',
        blocks: '512字节块数',
        supported: '支持的格式',
        value: '数值',
        ready: '已就绪',
        highCapacity: '高容量卡',
        switch18: '切换1.8V',
        reason: '结束原因',
        raw: '原始SCR',
        sdSpec: 'SD规范版本编码',
        dataAfterErase: '擦除后数据为一',
        security: '安全规范版本',
        busWidths: '支持的数据线宽度',
        spec3: '支持3.0及以上版本',
        extendedSecurity: '扩展安全能力',
        spec4: '支持4.0及以上版本',
        specExtension: '规范扩展版本',
        commandSupport: '支持的命令',
        vendorData: '制造商数据',
      },
      values: {},
    },
  },
  decoder: Registers,
})
