// SPDX-License-Identifier: Apache-2.0

/**
 * The SPI NOR decoder: the single-line command set of serial NOR flashes, read from the
 * transactions of an SPI stream. Implemented from the JEDEC SFDP standard (JESD216) and the
 * datasheets of common parts (Winbond W25Q, Macronix MX25L, Micron N25Q): an opcode, an
 * address of three or four bytes where the command takes one, dummy bytes, then data in one
 * direction, status register values or identification bytes.
 *
 * - The decoder reads one command per transaction: the first word on MOSI is the opcode; a
 *   transaction whose beginning was not seen is drained, since its first word is not the
 *   opcode; an opcode the decoder does not know is reported and the transaction skipped.
 * - The width of an address that follows the flash's address mode is three bytes, four, or
 *   follows Enter and Exit 4-Byte Address Mode and a software reset (`auto`); a gap returns the
 *   decoder to its configured width.
 * - Data goes out in batches of at most `DATA_BATCH` bytes, a batch as soon as it is full and
 *   the rest when the command ends; an incomplete command's bytes are flushed by its end.
 * - What the host must send is read with `item`, so that chip select released before it ends
 *   the operation `incomplete`; what may stop after any byte — the flash's answers, a program's
 *   further bytes — is read with `itemOrEnd`. A transaction cut off by the lower decoder, by a
 *   gap or by the end of the stream ends the operation `aborted`.
 */

import type { Bounded, Provenance, Tick, Transaction, bool, u32, u64, u8 } from 'uiharu'
import { BusDecoder, batch, defineBusDecoder, eventSchema, max, at as placedAt, over } from 'uiharu'

import { spiEvents, type SpiWord, type TxId } from './spi'

/** The most bytes one `data` or `id` event carries. */
export const DATA_BATCH: u32 = 32

/** An operation on the flash, numbered from 1 by the decoder. */
export type OpId = u64

/** The commands the decoder knows, named after what they do. */
export type Command =
  | 'write-enable'
  | 'write-disable'
  | 'volatile-sr-write-enable'
  | 'read-status'
  | 'read-status-2'
  | 'read-status-3'
  | 'write-status'
  | 'write-status-2'
  | 'write-status-3'
  | 'read'
  | 'fast-read'
  | 'read-4b'
  | 'fast-read-4b'
  | 'page-program'
  | 'page-program-4b'
  | 'sector-erase'
  | 'sector-erase-4b'
  | 'block-erase-32k'
  | 'block-erase-64k'
  | 'block-erase-64k-4b'
  | 'chip-erase'
  | 'deep-power-down'
  | 'release-power-down'
  | 'read-manufacturer-device-id'
  | 'read-jedec-id'
  | 'read-sfdp'
  | 'read-unique-id'
  | 'enter-4b-mode'
  | 'exit-4b-mode'
  | 'reset-enable'
  | 'reset'
  | 'program-erase-suspend'
  | 'program-erase-resume'
  | 'read-security-register'
  | 'program-security-register'
  | 'erase-security-register'
  | 'block-lock'
  | 'block-unlock'
  | 'global-block-lock'
  | 'global-block-unlock'

/** Which way data moves: into the flash on MOSI (`in`) or out to the host on MISO (`out`). */
export type Direction = 'in' | 'out'

/** Which identification an `id` event carries. */
export type IdScheme = 'jedec' | 'manufacturer-device' | 'unique' | 'electronic-signature'

/** How an operation ended. */
export type NorEnd =
  /** The command ran to its end: chip select was released after it was whole. */
  | 'complete'
  /** Chip select was released before the command was whole. */
  | 'incomplete'
  /** The samples ended or were lost while the command was under way. */
  | 'aborted'

/** What travels on an SPI NOR event stream. */
export type NorEvent =
  /** A command was recognised on SPI transaction `tx`. */
  | { kind: 'begin'; op: OpId; tx: TxId; command: Command }
  /** The command's address, most significant byte first on the bus, of `bytes` bytes. */
  | { kind: 'address'; op: OpId; address: u32; bytes: u8 }
  /** Dummy bytes were clocked; their values mean nothing. */
  | { kind: 'dummy'; op: OpId; bytes: u8 }
  /** Bytes read from or written to the flash, in order, `offset` counted from the operation's
   * first data byte. */
  | { kind: 'data'; op: OpId; direction: Direction; offset: u32; bytes: Bounded<u8, 32> }
  /** A status register value was read, or written by the host when `written`. */
  | { kind: 'status'; op: OpId; register: u8; value: u8; written: bool }
  /** Identification bytes the flash sent; a longer JEDEC ID read continues in the next one. */
  | { kind: 'id'; op: OpId; scheme: IdScheme; bytes: Bounded<u8, 32> }
  /** The operation ended. */
  | { kind: 'end'; op: OpId; reason: NorEnd }
  /** A transaction started with an opcode the decoder does not know; its words were skipped. */
  | { kind: 'unknown-command'; tx: TxId; opcode: u8 }

/** The schema of SPI NOR event streams. Version 2 carries the single-line command set. */
export const norEvents = eventSchema<NorEvent>('nor.events', 2)

/** How many address bytes the commands whose width follows the flash's address mode take. */
export type AddressBytes =
  /** Three: flashes of up to 16 MiB, and larger ones in their power-on mode. */
  | 'three'
  /** Four: a flash kept in four-byte address mode. */
  | 'four'
  /** Three until Enter 4-Byte Address Mode completes, four until Exit 4-Byte Address Mode or a
   * software reset completes, and three again after a gap. */
  | 'auto'

/** How the decoder reads the flash's commands. */
export interface SpiNorConfig {
  addressBytes: AddressBytes
}

/** Where a command's address width comes from. */
type AddressWidth = 'none' | 'three' | 'four' | 'mode'

/** What a command transfers after its address and dummy bytes. */
type Phase =
  /** Nothing: the command is whole. */
  | { kind: 'none' }
  /** Data from the flash on MISO, as many bytes as the host reads. */
  | { kind: 'data-out' }
  /** Data from the host on MOSI, at least one byte. */
  | { kind: 'data-in' }
  /** Values of status register `register` on MISO, as many as the host reads. */
  | { kind: 'read-status'; register: u8 }
  /** Values for status registers `first` to `last` on MOSI, at least one. */
  | { kind: 'write-status'; first: u8; last: u8 }
  /** Identification bytes on MISO: `len` of them, or as many as the host reads. */
  | { kind: 'id'; scheme: IdScheme; len: u8 | undefined }
  /** RES: nothing, or three dummy bytes and the one-byte electronic signature. */
  | { kind: 'signature' }

/** What a command shifts after its opcode. */
interface Layout {
  address: AddressWidth
  dummy: u8
  phase: Phase
}

/** The command `opcode` stands for, if the decoder knows it. */
function commandOf(opcode: u8): Command | undefined {
  switch (opcode) {
    case 0x06:
      return 'write-enable'
    case 0x04:
      return 'write-disable'
    case 0x50:
      return 'volatile-sr-write-enable'
    case 0x05:
      return 'read-status'
    case 0x35:
      return 'read-status-2'
    case 0x15:
      return 'read-status-3'
    case 0x01:
      return 'write-status'
    case 0x31:
      return 'write-status-2'
    case 0x11:
      return 'write-status-3'
    case 0x03:
      return 'read'
    case 0x0b:
      return 'fast-read'
    case 0x13:
      return 'read-4b'
    case 0x0c:
      return 'fast-read-4b'
    case 0x02:
      return 'page-program'
    case 0x12:
      return 'page-program-4b'
    case 0x20:
      return 'sector-erase'
    case 0x21:
      return 'sector-erase-4b'
    case 0x52:
      return 'block-erase-32k'
    case 0xd8:
      return 'block-erase-64k'
    case 0xdc:
      return 'block-erase-64k-4b'
    // Chip Erase answers to two opcodes.
    case 0xc7:
    case 0x60:
      return 'chip-erase'
    case 0xb9:
      return 'deep-power-down'
    case 0xab:
      return 'release-power-down'
    case 0x90:
      return 'read-manufacturer-device-id'
    case 0x9f:
      return 'read-jedec-id'
    case 0x5a:
      return 'read-sfdp'
    case 0x4b:
      return 'read-unique-id'
    case 0xb7:
      return 'enter-4b-mode'
    case 0xe9:
      return 'exit-4b-mode'
    case 0x66:
      return 'reset-enable'
    case 0x99:
      return 'reset'
    case 0x75:
      return 'program-erase-suspend'
    case 0x7a:
      return 'program-erase-resume'
    case 0x48:
      return 'read-security-register'
    case 0x42:
      return 'program-security-register'
    case 0x44:
      return 'erase-security-register'
    case 0x36:
      return 'block-lock'
    case 0x39:
      return 'block-unlock'
    case 0x7e:
      return 'global-block-lock'
    case 0x98:
      return 'global-block-unlock'
    default:
      return undefined
  }
}

function bare(phase: Phase): Layout {
  return { address: 'none', dummy: 0, phase }
}

function addressed(address: AddressWidth, dummy: u8, phase: Phase): Layout {
  return { address, dummy, phase }
}

/** What the command shifts after its opcode. */
function layoutOf(command: Command): Layout {
  switch (command) {
    case 'write-enable':
    case 'write-disable':
    case 'volatile-sr-write-enable':
    case 'chip-erase':
    case 'deep-power-down':
    case 'enter-4b-mode':
    case 'exit-4b-mode':
    case 'reset-enable':
    case 'reset':
    case 'program-erase-suspend':
    case 'program-erase-resume':
    case 'global-block-lock':
    case 'global-block-unlock':
      return bare({ kind: 'none' })
    case 'read-status':
      return bare({ kind: 'read-status', register: 1 })
    case 'read-status-2':
      return bare({ kind: 'read-status', register: 2 })
    case 'read-status-3':
      return bare({ kind: 'read-status', register: 3 })
    case 'write-status':
      return bare({ kind: 'write-status', first: 1, last: 2 })
    case 'write-status-2':
      return bare({ kind: 'write-status', first: 2, last: 2 })
    case 'write-status-3':
      return bare({ kind: 'write-status', first: 3, last: 3 })
    case 'read':
      return addressed('mode', 0, { kind: 'data-out' })
    case 'fast-read':
    case 'read-security-register':
      return addressed('mode', 1, { kind: 'data-out' })
    case 'read-4b':
      return addressed('four', 0, { kind: 'data-out' })
    case 'fast-read-4b':
      return addressed('four', 1, { kind: 'data-out' })
    case 'page-program':
    case 'program-security-register':
      return addressed('mode', 0, { kind: 'data-in' })
    case 'page-program-4b':
      return addressed('four', 0, { kind: 'data-in' })
    case 'sector-erase':
    case 'block-erase-32k':
    case 'block-erase-64k':
    case 'erase-security-register':
    case 'block-lock':
    case 'block-unlock':
      return addressed('mode', 0, { kind: 'none' })
    case 'sector-erase-4b':
    case 'block-erase-64k-4b':
      return addressed('four', 0, { kind: 'none' })
    case 'release-power-down':
      return bare({ kind: 'signature' })
    case 'read-manufacturer-device-id':
      return addressed('three', 0, { kind: 'id', scheme: 'manufacturer-device', len: 2 })
    case 'read-jedec-id':
      return bare({ kind: 'id', scheme: 'jedec', len: undefined })
    // JESD216 defines the SFDP read with a three-byte address in every address mode.
    case 'read-sfdp':
      return addressed('three', 1, { kind: 'data-out' })
    case 'read-unique-id':
      return addressed('none', 4, { kind: 'id', scheme: 'unique', len: 8 })
  }
}

/** The width in bytes of an address of `width` when the address mode is `mode` bytes. */
function addressWidth(width: AddressWidth, mode: u8): u8 {
  switch (width) {
    case 'none':
      return 0
    case 'three':
      return 3
    case 'four':
      return 4
    case 'mode':
      return mode
  }
}

/** What the current batch holds. */
type BatchOf = { kind: 'data'; direction: Direction } | { kind: 'id'; scheme: IdScheme }

function sameBatch(a: BatchOf, b: BatchOf): bool {
  switch (a.kind) {
    case 'data':
      return b.kind === 'data' && a.direction === b.direction
    case 'id':
      return b.kind === 'id' && a.scheme === b.scheme
  }
}

class SpiNor extends BusDecoder<SpiNorConfig, SpiWord, NorEvent, NorEnd> {
  // What the decoder knows of the flash from one transaction to the next: its address mode.
  /** Whether the commands that follow the mode take four address bytes now. */
  fourByte = this.config.addressBytes === 'four'
  /** Whether the last command was a complete Reset Enable, which arms Reset. */
  resetEnabled = false
  nextOp: u64 = 1
  /** The operation under way, until its end goes out. */
  open: OpId | undefined = undefined
  /** At most `DATA_BATCH` bytes waiting to go out. */
  readonly batch = batch<u8>(DATA_BATCH)
  batchOf: BatchOf = { kind: 'data', direction: 'out' }
  batchOffset: u32 = 0
  /** The data bytes of the open operation so far. */
  count: u32 = 0

  /** The batch being filled is held: progress inside the transaction stops at its first byte. */
  override hold(): Tick | undefined {
    return this.batch.at?.span.start
  }

  /** Forgets what the commands lost in a gap may have changed. */
  override onGap(): void {
    this.fourByte = this.config.addressBytes === 'four'
    this.resetEnabled = false
  }

  /** The width of an address that follows the mode. */
  addressBytes(): u8 {
    return this.fourByte ? 4 : 3
  }

  /** Follows a command that completed; `resetEnabled` says whether a complete Reset Enable
   * came directly before it. */
  completed(command: Command, resetEnabled: bool): void {
    const auto = this.config.addressBytes === 'auto'
    switch (command) {
      case 'enter-4b-mode':
        if (auto) this.fourByte = true
        break
      case 'exit-4b-mode':
        if (auto) this.fourByte = false
        break
      // A reset returns the flash to its power-on mode, which auto takes for three bytes.
      case 'reset':
        if (auto && resetEnabled) this.fourByte = false
        break
      case 'reset-enable':
        this.resetEnabled = true
        break
      default:
        break
    }
  }

  async begin(command: Command, tx: TxId): Promise<OpId> {
    const op = this.nextOp
    this.nextOp += 1
    this.open = op
    this.count = 0
    await this.out.emit({ kind: 'begin', op, tx, command })
    return op
  }

  /** Adds a byte to the current batch, which goes out once it holds `DATA_BATCH` bytes. */
  async push(op: OpId, of: BatchOf, byte: u8, where: Provenance): Promise<void> {
    if (!this.batch.isEmpty && !sameBatch(this.batchOf, of)) await this.flush(op, undefined)
    if (this.batch.isEmpty) {
      this.batchOf = of
      this.batchOffset = this.count
    }
    this.batch.push(byte, where)
    if (of.kind === 'data') this.count += 1
    if (this.batch.isFull) await this.flush(op, undefined)
  }

  /** Sends the batch, if any. A batch is settled when it is decided: by its last byte when it
   * is full or whole (`decided` is `undefined`), by the end that decided it otherwise, so that
   * the stream's events settle in order. */
  async flush(op: OpId, decided: Tick | undefined): Promise<void> {
    if (this.batch.isEmpty) return
    const taken = this.batch.take()
    let where = taken.at ?? placedAt(decided ?? 0)
    if (decided !== undefined) {
      where = over(where.span.start, where.span.end, max(where.settled, decided))
    }
    const of = this.batchOf
    switch (of.kind) {
      case 'data':
        await this.out.emit(
          {
            kind: 'data',
            op,
            direction: of.direction,
            offset: this.batchOffset,
            bytes: taken.items,
          },
          where,
        )
        break
      case 'id':
        await this.out.emit({ kind: 'id', op, scheme: of.scheme, bytes: taken.items }, where)
        break
    }
  }

  async end(op: OpId, reason: NorEnd, where: Provenance): Promise<void> {
    await this.flush(op, where.settled)
    this.open = undefined
    await this.out.emit({ kind: 'end', op, reason }, where)
  }

  /** Closes what the body left open, as the transaction's end calls for. */
  override async recover(reason: NorEnd, where: Provenance): Promise<void> {
    if (this.open !== undefined) await this.end(this.open, reason, where)
  }

  /** The command one transaction carries. */
  override async transaction(tx: Transaction<SpiWord>): Promise<void> {
    const opcode = (await tx.item()).mosi!
    // Any command after Reset Enable disarms it.
    const resetEnabled = this.resetEnabled
    this.resetEnabled = false
    const command = commandOf(opcode)
    if (command === undefined) {
      await this.out.emit({ kind: 'unknown-command', tx: tx.id, opcode })
      await tx.drain()
      return
    }
    const op = await this.begin(command, tx.id)
    const layout = layoutOf(command)
    const width = addressWidth(layout.address, this.addressBytes())
    if (width > 0) {
      // Most significant byte first. Pausing between the reads keeps the bytes read so far:
      // they are the state of the body.
      let address: u32 = 0
      for (let i = 0; i < width; i++) address = (address << 8) | (await tx.item()).mosi!
      await this.out.emit({ kind: 'address', op, address, bytes: width })
    }
    if (layout.dummy > 0) {
      for (let i = 0; i < layout.dummy; i++) await tx.item()
      await this.out.emit({ kind: 'dummy', op, bytes: layout.dummy })
    }
    if (!(await this.phase(tx, op, layout.phase))) {
      // The command is whole and takes nothing more; what follows is skipped.
      await tx.drain()
    }
    await this.end(op, 'complete', this.consumed())
    this.completed(command, resetEnabled)
  }

  /** Reads what the command transfers after its address and dummy bytes. Returns whether it
   * found the transaction's normal end. */
  async phase(tx: Transaction<SpiWord>, op: OpId, phase: Phase): Promise<bool> {
    switch (phase.kind) {
      case 'none':
        return false
      case 'data-out':
        for (;;) {
          const word = await tx.itemOrEnd()
          if (word === undefined) return true
          await this.push(op, { kind: 'data', direction: 'out' }, word.miso!, this.consumed())
        }
      case 'data-in': {
        const data: BatchOf = { kind: 'data', direction: 'in' }
        const first = (await tx.item()).mosi!
        await this.push(op, data, first, this.consumed())
        for (;;) {
          const word = await tx.itemOrEnd()
          if (word === undefined) return true
          await this.push(op, data, word.mosi!, this.consumed())
        }
      }
      case 'read-status':
        for (;;) {
          const word = await tx.itemOrEnd()
          if (word === undefined) return true
          const value = word.miso!
          await this.out.emit({
            kind: 'status',
            op,
            register: phase.register,
            value,
            written: false,
          })
        }
      case 'write-status': {
        const value = (await tx.item()).mosi!
        await this.out.emit({ kind: 'status', op, register: phase.first, value, written: true })
        for (let register = phase.first + 1; register <= phase.last; register++) {
          const word = await tx.itemOrEnd()
          if (word === undefined) return true
          await this.out.emit({
            kind: 'status',
            op,
            register,
            value: word.mosi!,
            written: true,
          })
        }
        return false
      }
      case 'id': {
        // `undefined` reads as many bytes as the host clocks, in batches.
        let left = phase.len
        for (;;) {
          if (left === 0) {
            await this.flush(op, undefined)
            return false
          }
          const word = await tx.itemOrEnd()
          if (word === undefined) return true
          await this.push(op, { kind: 'id', scheme: phase.scheme }, word.miso!, this.consumed())
          if (left !== undefined) left -= 1
        }
      }
      case 'signature': {
        // Release from power-down alone, or three dummy bytes and the signature.
        if ((await tx.itemOrEnd()) === undefined) return true
        for (let i = 1; i < 3; i++) await tx.item()
        await this.out.emit({ kind: 'dummy', op, bytes: 3 })
        const word = await tx.itemOrEnd()
        if (word === undefined) return true
        const id: BatchOf = { kind: 'id', scheme: 'electronic-signature' }
        await this.push(op, id, word.miso!, this.consumed())
        await this.flush(op, undefined)
        return false
      }
    }
  }
}

/** The SPI NOR decoder node: `spi.events` in, `nor.events` out. It requires bytes, most
 * significant bit first, framed by chip select, with both data lines. */
export const spiNor = defineBusDecoder({
  name: 'spi-nor',
  category: 'device',
  config: { defaults: { addressBytes: 'three' } },
  input: spiEvents,
  requires: () => ({
    wordBits: 8,
    bitOrder: 'msb-first',
    cs: true,
    mosi: true,
    miso: true,
  }),
  output: norEvents,
  props: (config) => ({ addressBytes: config.addressBytes }),
  locales: {
    'en-US': {
      name: 'SPI NOR flash',
      summary: 'Commands a host sends to a serial NOR flash over single-line SPI',
      fields: { addressBytes: 'Address bytes' },
      values: { addressBytes: { three: '3 bytes', four: '4 bytes', auto: 'Auto' } },
      events: {
        begin: 'Begin',
        address: 'Address',
        dummy: 'Dummy bytes',
        data: 'Data',
        status: 'Status register',
        id: 'ID',
        end: 'End',
        'unknown-command': 'Unknown command',
      },
      eventFields: {
        op: 'Operation',
        tx: 'Transaction',
        command: 'Command',
        address: 'Address',
        bytes: 'Bytes',
        direction: 'Direction',
        offset: 'Offset',
        register: 'Register',
        value: 'Value',
        written: 'Written',
        scheme: 'ID type',
        reason: 'Reason',
        opcode: 'Opcode',
      },
    },
    'zh-CN': {
      name: 'SPI NOR 闪存',
      summary: '读取主机通过单线 SPI 发给串行 NOR 闪存的命令',
      fields: { addressBytes: '地址字节数' },
      values: { addressBytes: { three: '3 字节', four: '4 字节', auto: '自动' } },
      events: {
        begin: '开始',
        address: '地址',
        dummy: 'Dummy 字节',
        data: '数据',
        status: '状态寄存器',
        id: 'ID',
        end: '结束',
        'unknown-command': '未知命令',
      },
      eventFields: {
        op: '操作号',
        tx: '事务号',
        command: '命令',
        address: '地址',
        bytes: '字节',
        direction: '方向',
        offset: '偏移',
        register: '寄存器',
        value: '值',
        written: '写入',
        scheme: 'ID 类型',
        reason: '原因',
        opcode: '操作码',
      },
    },
  },
  partial: 'drain',
  recovery: {
    incomplete: 'incomplete',
    truncated: 'aborted',
    missing: 'aborted',
    gap: 'aborted',
    end: 'aborted',
  },
  decoder: SpiNor,
})
