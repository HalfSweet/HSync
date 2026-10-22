// SPDX-License-Identifier: Apache-2.0
/** SMBus3.2 transaction profiles, SMIF specification2022-01-12 §§6.4–6.5.
 * Independent I2C consumer; one PEC lookbehind byte and one <=8-byte scalar.
 * No clock-low timeout/electrical conformance is inferred from byte events. */
import type { ItemOf, Transaction, Provenance, Tick, bool, u8, u16, u32, u64 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, join, over } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'
type BusItem = ItemOf<typeof i2cEvents>
export type Mode =
  | 'raw'
  | 'quick'
  | 'send-byte'
  | 'receive-byte'
  | 'write-byte'
  | 'write-word'
  | 'read-byte'
  | 'read-word'
  | 'process-call'
  | 'block-write'
  | 'block-read'
  | 'block-process-call'
  | 'host-notify'
  | 'write-32'
  | 'read-32'
  | 'write-64'
  | 'read-64'
export interface Config {
  address: u16 | undefined
  mode: Mode
  pec: bool
  maxBytes: u16
  maxBlockBytes: u8
}
export type End = 'complete' | 'incomplete' | 'aborted' | 'bad-pec' | 'invalid' | 'nack' | 'limit'
export type Event =
  | { kind: 'begin'; tx: u64; address: u16; mode: Mode; pec: bool }
  | { kind: 'address'; tx: u64; address: u16; read: bool; ack: bool }
  | { kind: 'byte'; tx: u64; read: bool; index: u16; value: u8; ack: bool }
  | { kind: 'command'; tx: u64; value: u8 }
  | { kind: 'count'; tx: u64; read: bool; value: u8; valid: bool }
  | { kind: 'value'; tx: u64; read: bool; bytes: u8; low: u32; high: u32 }
  | { kind: 'notify'; tx: u64; device: u8 }
  | { kind: 'pec'; tx: u64; value: u8; expected: u8; ok: bool }
  | { kind: 'end'; tx: u64; mode: Mode; reason: End; lengthOk: bool; valid: bool }
export const events = eventSchema<Event>('smbus.events', 1).roles({
  begin: 'framing',
  address: 'address',
  byte: 'data',
  command: 'command',
  count: { role: 'control', errorWhen: { valid: false } },
  value: 'data',
  notify: 'control',
  pec: { role: 'check', errorWhen: { ok: false } },
  end: { role: 'framing', errorWhen: { valid: false } },
})
function validate(c: Config): string | undefined {
  if (c.address !== undefined && c.address > 127) return 'smbus.config.address'
  if (c.maxBytes < 2 || c.maxBytes > 1024) return 'smbus.config.bytes'
  if (c.maxBlockBytes === 0) return 'smbus.config.block'
  return undefined
}
function crc8(crc: u8, byte: u8): u8 {
  let v: u8 = crc ^ byte
  for (let bit = 0; bit < 8; bit += 1) v = (v & 128) !== 0 ? ((v << 1) ^ 7) & 255 : (v << 1) & 255
  return v
}
function pecEnabled(c: Config): bool {
  return c.pec && c.mode !== 'quick' && c.mode !== 'host-notify'
}
function readMode(m: Mode): bool {
  return (
    m === 'read-byte' ||
    m === 'read-word' ||
    m === 'read-32' ||
    m === 'read-64' ||
    m === 'process-call' ||
    m === 'block-read' ||
    m === 'block-process-call'
  )
}
function hasCommand(m: Mode): bool {
  return (
    m !== 'raw' && m !== 'quick' && m !== 'send-byte' && m !== 'receive-byte' && m !== 'host-notify'
  )
}
function readWidth(m: Mode): u8 {
  if (m === 'receive-byte' || m === 'read-byte') return 1
  if (m === 'read-word' || m === 'process-call') return 2
  if (m === 'read-32') return 4
  if (m === 'read-64') return 8
  return 0
}
function writeWidth(m: Mode): u8 {
  if (m === 'send-byte' || m === 'write-byte') return 1
  if (m === 'write-word' || m === 'process-call' || m === 'host-notify') return 2
  if (m === 'write-32') return 4
  if (m === 'write-64') return 8
  return 0
}
class Smbus extends BusDecoder<Config, BusItem, Event, End> {
  open = false
  tx: u64 = 0
  first: Provenance | undefined = undefined
  target: u16 = 0
  read = false
  segments: u8 = 0
  directions: u8 = 0
  wireBytes: u16 = 0
  writeCount: u16 = 0
  readCount: u16 = 0
  writeBlock: u16 = 0
  readBlock: u16 = 0
  haveWriteBlock = false
  haveReadBlock = false
  invalid = false
  nack = false
  crc: u8 = 0
  tail: u8 | undefined = undefined
  tailAt: Provenance | undefined = undefined
  tailRead = false
  tailAck = false
  readTerminated = false
  wordAt: Provenance | undefined = undefined
  wordCount: u8 = 0
  low: u32 = 0
  high: u32 = 0
  override hold(): Tick | undefined {
    const first = this.first
    if (first !== undefined) return first.span.start
    return undefined
  }
  reset(tx: u64, address: u16, at: Provenance): void {
    this.open = true
    this.tx = tx
    this.target = address
    this.first = at
    this.segments = 0
    this.directions = 0
    this.wireBytes = 0
    this.writeCount = 0
    this.readCount = 0
    this.writeBlock = 0
    this.readBlock = 0
    this.haveWriteBlock = false
    this.haveReadBlock = false
    this.invalid = false
    this.nack = false
    this.crc = 0
    this.tail = undefined
    this.tailAt = undefined
    this.tailRead = false
    this.tailAck = false
    this.readTerminated = false
    this.wordAt = undefined
    this.wordCount = 0
    this.low = 0
    this.high = 0
  }
  async consume(byte: u8, read: bool, ack: bool, at: Provenance): Promise<void> {
    this.crc = crc8(this.crc, byte)
    const index = read ? this.readCount : this.writeCount
    if (read) this.readCount += 1
    else this.writeCount += 1
    if (!read && !ack) this.nack = true
    await this.out.emit({ kind: 'byte', tx: this.tx, read, index, value: byte, ack }, at)
    const mode = this.config.mode
    if (!read && index === 0 && hasCommand(mode)) {
      await this.out.emit({ kind: 'command', tx: this.tx, value: byte }, at)
      return
    }
    if (!read && index === 0 && mode === 'host-notify') {
      await this.out.emit({ kind: 'notify', tx: this.tx, device: byte >> 1 }, at)
      return
    }
    const count = read
      ? (mode === 'block-read' || mode === 'block-process-call') && index === 0
      : (mode === 'block-write' || mode === 'block-process-call') && index === 1
    if (count) {
      if (read) {
        this.haveReadBlock = true
        this.readBlock = byte
      } else {
        this.haveWriteBlock = true
        this.writeBlock = byte
      }
      const valid = byte <= this.config.maxBlockBytes
      if (!valid) this.invalid = true
      await this.out.emit({ kind: 'count', tx: this.tx, read, value: byte, valid }, at)
      return
    }
    const width = read ? readWidth(mode) : writeWidth(mode)
    const start: u16 = read || mode === 'send-byte' ? 0 : 1
    if (width === 0 || index < start || index >= start + width) return
    if (index === start) {
      this.wordAt = at
      this.wordCount = 0
      this.low = 0
      this.high = 0
    }
    if (this.wordCount < 4) this.low |= (byte as u32) << ((this.wordCount as u64) * 8)
    else this.high |= (byte as u32) << (((this.wordCount - 4) as u64) * 8)
    this.wordCount += 1
    if (this.wordCount === width) {
      const first = this.wordAt
      let placed = at
      if (first !== undefined) placed = join(first, at)
      await this.out.emit(
        { kind: 'value', tx: this.tx, read, bytes: width, low: this.low, high: this.high },
        placed,
      )
      this.wordAt = undefined
      this.wordCount = 0
    }
  }
  async flush(): Promise<void> {
    const tail = this.tail
    const at = this.tailAt
    if (tail !== undefined && at !== undefined)
      await this.consume(tail, this.tailRead, this.tailAck, at)
    this.tail = undefined
    this.tailAt = undefined
  }
  lengthOk(): bool {
    const mode = this.config.mode
    if (mode === 'raw') return this.segments >= 1 && this.segments <= 2
    if (mode === 'quick')
      return this.segments === 1 && this.writeCount === 0 && this.readCount === 0
    const directRead = mode === 'receive-byte'
    if (
      this.segments !== (readMode(mode) ? 2 : 1) ||
      this.directions !== (readMode(mode) || directRead ? 1 : 0)
    )
      return false
    if (mode === 'host-notify' && this.target !== 8) return false
    let expectedWrite: u16 = hasCommand(mode) ? 1 : 0
    let expectedRead: u16 = readWidth(mode)
    if (mode === 'block-write' || mode === 'block-process-call') {
      if (!this.haveWriteBlock) return false
      expectedWrite = 2 + this.writeBlock
    } else expectedWrite += writeWidth(mode)
    if (mode === 'block-read' || mode === 'block-process-call') {
      if (!this.haveReadBlock) return false
      expectedRead = 1 + this.readBlock
    }
    if (mode === 'host-notify') expectedWrite = 3
    if (mode === 'block-process-call' && this.writeBlock + this.readBlock > 255) return false
    return this.writeCount === expectedWrite && this.readCount === expectedRead
  }
  async close(reason: End, where: Provenance): Promise<void> {
    if (!this.open) return
    let ending = reason
    let checked = true
    const tail = this.tail
    const tailAt = this.tailAt
    const pec = pecEnabled(this.config) && this.wireBytes !== 0
    if (reason === 'complete' && pec) {
      if (tail === undefined) checked = false
      else if (tailAt === undefined) checked = false
      else {
        checked = tail === this.crc
        const first = this.first
        let placed = over(tailAt.span.start, tailAt.span.end, where.settled)
        if (first !== undefined) placed = over(first.span.start, tailAt.span.end, where.settled)
        await this.out.emit(
          { kind: 'pec', tx: this.tx, value: tail, expected: this.crc, ok: checked },
          placed,
        )
        if (!this.tailRead && !this.tailAck) this.nack = true
        if (this.tailRead && this.tailAck) this.invalid = true
      }
      this.tail = undefined
      this.tailAt = undefined
    } else {
      await this.flush()
      if (reason === 'complete' && this.read && this.tailAck) this.invalid = true
    }
    const lengthOk = this.lengthOk()
    if (ending === 'complete') {
      if (this.nack) ending = 'nack'
      else if (!lengthOk) ending = 'incomplete'
      else if (this.invalid) ending = 'invalid'
      else if (!checked) ending = 'bad-pec'
    }
    await this.out.emit(
      {
        kind: 'end',
        tx: this.tx,
        mode: this.config.mode,
        reason: ending,
        lengthOk,
        valid: ending === 'complete' && lengthOk && checked && !this.invalid && !this.nack,
      },
      where,
    )
    this.open = false
    this.first = undefined
    this.tailAt = undefined
    this.wordAt = undefined
  }
  override async recover(reason: End, where: Provenance): Promise<void> {
    await this.close(reason, where)
  }
  override async transaction(tx: Transaction<BusItem>): Promise<void> {
    let active = false
    for (;;) {
      const item = await tx.itemOrEnd()
      if (item === undefined) {
        await this.close('complete', this.consumed())
        return
      }
      const where = this.consumed()
      switch (item.kind) {
        case 'address': {
          if (item.tenBit) {
            if (this.open) this.invalid = true
            active = false
            continue
          }
          if (!this.open) {
            if (this.config.address !== undefined && this.config.address !== item.address) {
              active = false
              continue
            }
            this.reset(tx.id, item.address, where)
            await this.out.emit(
              {
                kind: 'begin',
                tx: this.tx,
                address: item.address,
                mode: this.config.mode,
                pec: pecEnabled(this.config),
              },
              where,
            )
          }
          if (this.readTerminated) this.invalid = true
          await this.flush()
          if (this.segments >= 2) this.invalid = true
          else {
            this.segments += 1
            this.directions = (this.directions << 1) | (item.read ? 1 : 0)
          }
          if (item.address !== this.target) this.invalid = true
          if (!item.ack) this.nack = true
          this.read = item.read
          active = true
          // eslint-disable-next-line @typescript-eslint/no-unnecessary-type-assertion -- The DSL IR requires an explicit u8 narrowing at this boundary.
          this.crc = crc8(this.crc, ((item.address << 1) | (item.read ? 1 : 0)) as u8)
          await this.out.emit(
            { kind: 'address', tx: this.tx, address: item.address, read: item.read, ack: item.ack },
            where,
          )
          continue
        }
        case 'restart':
          await this.flush()
          if (item.droppedBits !== 0) this.invalid = true
          active = false
          continue
        case 'data': {
          if (!active || !this.open) continue
          if (this.wireBytes === this.config.maxBytes) {
            await this.close('limit', where)
            await tx.drain()
            return
          }
          if (this.readTerminated) this.invalid = true
          if (this.read && !item.ack) this.readTerminated = true
          await this.flush()
          this.tail = item.byte
          this.tailAt = where
          this.tailRead = this.read
          this.tailAck = item.ack
          this.wireBytes += 1
          continue
        }
      }
    }
  }
}
export const smbus = defineBusDecoder({
  name: 'smbus',
  category: 'bus',
  config: {
    defaults: { address: undefined, mode: 'raw', pec: true, maxBytes: 512, maxBlockBytes: 255 },
    validate,
  },
  input: i2cEvents,
  requires: () => ({ bus: 'i2c', framing: 'start-stop' }),
  output: events,
  props: (config) => ({ pec: pecEnabled(config) }),
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
      name: 'SMBus transactions',
      summary: 'Explicit byte, word, block and Host Notify profiles with optional PEC',
      fields: {
        address: 'Target address filter',
        mode: 'Transaction profile',
        pec: 'Final PEC byte',
        maxBytes: 'Maximum message bytes',
        maxBlockBytes: 'Maximum block count',
      },
      values: {
        mode: {
          raw: 'Raw transaction',
          quick: 'Quick command',
          'send-byte': 'Send byte',
          'receive-byte': 'Receive byte',
          'write-byte': 'Write byte',
          'write-word': 'Write word',
          'read-byte': 'Read byte',
          'read-word': 'Read word',
          'process-call': 'Process call',
          'block-write': 'Block write',
          'block-read': 'Block read',
          'block-process-call': 'Block process call',
          'host-notify': 'Host Notify',
          'write-32': 'Write 32 bits',
          'read-32': 'Read 32 bits',
          'write-64': 'Write 64 bits',
          'read-64': 'Read 64 bits',
        },
      },
      events: {
        begin: 'Message',
        address: 'Address',
        byte: 'Payload byte',
        command: 'Command',
        count: 'Block count',
        value: 'Scalar value',
        notify: 'Host Notify',
        pec: 'PEC',
        end: 'Message end',
      },
      eventFields: {
        tx: 'Transaction',
        address: 'Address',
        mode: 'Profile',
        pec: 'PEC enabled',
        read: 'Read',
        ack: 'Acknowledged',
        index: 'Byte index',
        value: 'Value',
        bytes: 'Width in bytes',
        low: 'Low 32 bits',
        high: 'High 32 bits',
        device: 'Notifying address',
        expected: 'Expected PEC',
        ok: 'PEC valid',
        reason: 'End reason',
        lengthOk: 'Length valid',
        valid: 'Transaction valid',
      },
    },
    'zh-CN': {
      name: 'SMBus事务',
      summary: '显式字节、字、块和主机通知格式及可选PEC校验',
      fields: {
        address: '目标地址筛选',
        mode: '事务格式',
        pec: '末尾PEC字节',
        maxBytes: '消息字节上限',
        maxBlockBytes: '块计数上限',
      },
      values: {
        mode: {
          raw: '原始事务',
          quick: '快速命令',
          'send-byte': '发送字节',
          'receive-byte': '接收字节',
          'write-byte': '写字节',
          'write-word': '写字',
          'read-byte': '读字节',
          'read-word': '读字',
          'process-call': '处理调用',
          'block-write': '块写入',
          'block-read': '块读取',
          'block-process-call': '块处理调用',
          'host-notify': '主机通知',
          'write-32': '写32位',
          'read-32': '读32位',
          'write-64': '写64位',
          'read-64': '读64位',
        },
      },
      events: {
        begin: '消息',
        address: '地址',
        byte: '数据字节',
        command: '命令',
        count: '块计数',
        value: '标量值',
        notify: '主机通知',
        pec: 'PEC',
        end: '消息结束',
      },
      eventFields: {
        tx: '事务',
        address: '地址',
        mode: '格式',
        pec: '启用PEC',
        read: '读取',
        ack: '已应答',
        index: '字节索引',
        value: '数值',
        bytes: '字节宽度',
        low: '低32位',
        high: '高32位',
        device: '通知设备地址',
        expected: '预期PEC',
        ok: 'PEC有效',
        reason: '结束原因',
        lengthOk: '长度有效',
        valid: '事务有效',
      },
    },
  },
  decoder: Smbus,
})
