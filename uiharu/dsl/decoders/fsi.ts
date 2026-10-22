// SPDX-License-Identifier: Apache-2.0
/** OpenFSI1.0.0 packet transport, OpenPOWER Foundation2016, chapters3–4.
 * Original bounded parser. CRC covers logical start and all payload bits.
 * Wire facts cross-checked with RaptorEngineering2020 AGPL reference, no copied body. */
import type { Channel, Tick, Range, Provenance, bool, u8, u16, u32, u64 } from 'uiharu'
import {
  LogicDecoder,
  defineLogicDecoder,
  eventSchema,
  mask,
  max,
  atLeast,
  fromBool,
  one,
  over,
  join,
} from 'uiharu'
export interface Config {
  clock: Channel
  data: Channel
  inverted: bool
  turnaround: u8
  responseTimeout: u16
  startAligned: bool
}
export type Command =
  'absolute' | 'relative' | 'same' | 'data-poll' | 'error-poll' | 'interrupt-poll' | 'terminate'
export type Status = 'ack' | 'busy' | 'error-a' | 'error-crc' | 'interrupt'
export type End =
  'ack' | 'busy' | 'error' | 'bad-crc' | 'timeout' | 'aborted' | 'invalid' | 'unknown-context'
export type Phase =
  | 'unsynced'
  | 'idle'
  | 'host-id'
  | 'prefix'
  | 'direction'
  | 'sign'
  | 'address'
  | 'size'
  | 'host-data'
  | 'crc-host'
  | 'tar-response'
  | 'response-wait'
  | 'slave-id'
  | 'status'
  | 'slave-data'
  | 'interrupt'
  | 'crc-slave'
  | 'tar-command'
export type Event =
  | { kind: 'break'; cycles: u16 }
  | {
      kind: 'command'
      packet: u64
      exchange: u64
      slave: u8
      command: Command
      read: bool
      bytes: u8
      address: u32 | undefined
      related: bool
    }
  | { kind: 'response'; packet: u64; exchange: u64; slave: u8; status: Status; related: bool }
  | { kind: 'byte'; packet: u64; exchange: u64; read: bool; index: u8; value: u8 }
  | { kind: 'interrupt'; packet: u64; levels: u8; dma: u8 }
  | { kind: 'crc'; packet: u64; host: bool; received: u8; expected: u8; ok: bool }
  | { kind: 'end'; packet: u64; exchange: u64; reason: End; success: bool }
export const events = eventSchema<Event>('fsi.events', 1).roles({
  break: 'control',
  command: 'command',
  response: 'handshake',
  byte: 'data',
  interrupt: 'control',
  crc: { role: 'check', errorWhen: { ok: false } },
  end: 'framing',
})
function crc4(crc: u8, bit: u8): u8 {
  const top: u8 = ((crc >> 3) ^ bit) & 1
  return ((crc << 1) & 15) ^ (top !== 0 ? 7 : 0)
}
function validate(c: Config): string | undefined {
  if (c.clock >= 64 || c.data >= 64 || c.clock === c.data) return 'fsi.config.channels'
  if (c.turnaround === 0 || c.turnaround > 32 || c.responseTimeout === 0) return 'fsi.config.timing'
  return undefined
}
class Fsi extends LogicDecoder<Config, Event> {
  readonly watch = mask(this.config.clock)
  phase: Phase = this.config.startAligned ? 'idle' : 'unsynced'
  first: Provenance | undefined = undefined
  breakFirst: Tick | undefined = undefined
  breakRun: u16 = 0
  breaking = false
  packet: u64 = 0
  exchange: u64 = 0
  slave: u8 = 0
  responseSlave: u8 = 0
  command: Command = 'absolute'
  status: Status = 'ack'
  read = false
  bytes: u8 = 0
  address: u32 | undefined = undefined
  related = false
  bits: u8 = 0
  value: u32 = 0
  need: u8 = 0
  negative = false
  rawAddress: u32 = 0
  byteIndex: u8 = 0
  byteValue: u8 = 0
  byteBits: u8 = 0
  byteFirst: Provenance | undefined = undefined
  crc: u8 = 0
  expected: u8 = 0
  received: u8 = 0
  hostOk = false
  responseRelated = false
  tar: u8 = 0
  waited: u16 = 0
  pending = false
  pendingExchange: u64 = 0
  pendingSlave: u8 = 0
  pendingRead = false
  pendingBytes: u8 = 0
  pendingAddress: u32 | undefined = undefined
  cache0: u32 | undefined = undefined
  cache1: u32 | undefined = undefined
  cache2: u32 | undefined = undefined
  cache3: u32 | undefined = undefined
  override hold(): Tick | undefined {
    const first = this.first
    const low = this.breakFirst
    if (first !== undefined) {
      if (low !== undefined && low < first.span.start) return low
      return first.span.start
    }
    return low
  }
  forget(): void {
    this.pending = false
    this.cache0 = undefined
    this.cache1 = undefined
    this.cache2 = undefined
    this.cache3 = undefined
  }
  cache(id: u8): u32 | undefined {
    switch (id) {
      case 0:
        return this.cache0
      case 1:
        return this.cache1
      case 2:
        return this.cache2
      default:
        return this.cache3
    }
  }
  setCache(id: u8, value: u32 | undefined): void {
    switch (id) {
      case 0:
        this.cache0 = value
        return
      case 1:
        this.cache1 = value
        return
      case 2:
        this.cache2 = value
        return
      default:
        this.cache3 = value
        return
    }
  }
  override async onGap(range: Range): Promise<void> {
    await this.cut(range.start)
    this.breakRun = 0
    this.breakFirst = undefined
    this.breaking = false
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
    this.breakFirst = undefined
  }
  async cut(tick: Tick): Promise<void> {
    if (this.first !== undefined || this.phase === 'response-wait' || this.phase === 'tar-response')
      await this.out.emit(
        {
          kind: 'end',
          packet: this.packet,
          exchange: this.exchange,
          reason: 'aborted',
          success: false,
        },
        over(tick, tick, tick),
      )
    this.phase = 'unsynced'
    this.first = undefined
    this.byteFirst = undefined
    this.forget()
  }
  async end(reason: End, success: bool, at: Provenance): Promise<void> {
    await this.out.emit(
      { kind: 'end', packet: this.packet, exchange: this.exchange, reason, success },
      at,
    )
    this.first = undefined
    this.byteFirst = undefined
    this.tar = this.config.turnaround
    this.phase = 'tar-command'
  }
  startCrc(host: bool): void {
    this.expected = this.crc
    this.received = 0
    this.bits = 0
    this.phase = host ? 'crc-host' : 'crc-slave'
  }
  async hostHeader(at: Provenance): Promise<void> {
    this.related =
      this.command === 'data-poll'
        ? this.pending && this.pendingSlave === this.slave
        : this.command === 'absolute' || this.command === 'relative' || this.command === 'same'
    if (this.command === 'data-poll') {
      this.exchange = this.related ? this.pendingExchange : this.packet
      this.read = this.related && this.pendingRead
      this.bytes = this.related ? this.pendingBytes : 0
      this.address = this.related ? this.pendingAddress : undefined
    } else {
      this.exchange = this.packet
      if (this.command !== 'error-poll') this.pending = false
    }
    const first = this.first
    const placed = first === undefined ? at : join(first, at)
    await this.out.emit(
      {
        kind: 'command',
        packet: this.packet,
        exchange: this.exchange,
        slave: this.slave,
        command: this.command,
        read: this.read,
        bytes: this.bytes,
        address: this.address,
        related: this.related,
      },
      placed,
    )
    if (
      !this.read &&
      this.bytes !== 0 &&
      (this.command === 'absolute' || this.command === 'relative' || this.command === 'same')
    ) {
      this.phase = 'host-data'
      this.byteBits = 0
      this.byteValue = 0
      this.byteIndex = 0
    } else this.startCrc(true)
  }
  async dataBit(bit: u8, at: Provenance, host: bool): Promise<void> {
    if (this.byteBits === 0) this.byteFirst = at
    this.byteValue = (this.byteValue << 1) | bit
    this.byteBits += 1
    if (this.byteBits === 8) {
      const first = this.byteFirst
      const placed = first === undefined ? at : join(first, at)
      await this.out.emit(
        {
          kind: 'byte',
          packet: this.packet,
          exchange: this.exchange,
          read: !host,
          index: this.byteIndex,
          value: this.byteValue,
        },
        placed,
      )
      this.byteIndex += 1
      this.byteBits = 0
      this.byteValue = 0
      if (this.byteIndex === this.bytes) this.startCrc(host)
    }
  }
  async hostBit(bit: u8, at: Provenance): Promise<void> {
    if (this.phase === 'idle') {
      if (bit === 0) return
      this.packet += 1
      this.first = at
      this.phase = 'host-id'
      this.bits = 0
      this.value = 0
      this.crc = crc4(0, 1)
      this.hostOk = false
      this.read = false
      this.bytes = 0
      this.address = undefined
      return
    }
    this.crc = crc4(this.crc, bit)
    if (this.phase === 'host-id') {
      this.value = (this.value << 1) | bit
      this.bits += 1
      if (this.bits === 2) {
        this.slave = this.value as u8
        this.phase = 'prefix'
        this.bits = 0
        this.value = 0
      }
      return
    }
    if (this.phase === 'prefix') {
      this.value = (this.value << 1) | bit
      this.bits += 1
      if (this.bits === 2 && this.value === 3) {
        this.command = 'same'
        this.need = 2
        this.phase = 'direction'
        return
      }
      if (this.bits !== 3) return
      if (this.value === 4) {
        this.command = 'absolute'
        this.need = 21
        this.phase = 'direction'
      } else if (this.value === 5) {
        this.command = 'relative'
        this.need = 8
        this.phase = 'direction'
      } else if (this.value === 1 || this.value === 2 || this.value === 3) {
        this.command =
          this.value === 1 ? 'interrupt-poll' : this.value === 2 ? 'data-poll' : 'error-poll'
        await this.hostHeader(at)
      } else {
        await this.end('invalid', false, at)
        this.phase = 'unsynced'
        this.forget()
      }
      return
    }
    if (this.phase === 'direction') {
      this.read = bit !== 0
      this.bits = 0
      this.rawAddress = 0
      this.negative = false
      this.phase = this.command === 'relative' ? 'sign' : 'address'
      return
    }
    if (this.phase === 'sign') {
      this.negative = bit !== 0
      this.phase = 'address'
      return
    }
    if (this.phase === 'address') {
      this.rawAddress = (this.rawAddress << 1) | bit
      this.bits += 1
      if (this.bits === this.need) this.phase = 'size'
      return
    }
    if (this.phase === 'size') {
      if (this.command === 'same' && this.read && this.rawAddress === 3 && bit !== 0) {
        this.command = 'terminate'
        this.read = false
        this.bytes = 0
        this.pending = false
        this.setCache(this.slave, undefined)
        await this.hostHeader(at)
        return
      }
      if (bit === 0) this.bytes = 1
      else if ((this.rawAddress & 3) === 1) this.bytes = 4
      else if ((this.rawAddress & 1) === 0) this.bytes = 2
      else {
        await this.end('invalid', false, at)
        this.phase = 'unsynced'
        this.forget()
        return
      }
      if (this.command === 'absolute') this.address = this.rawAddress
      else {
        const base = this.cache(this.slave)
        if (base !== undefined)
          this.address =
            this.command === 'same'
              ? base | this.rawAddress
              : this.negative
                ? (base + 0x200000 - this.rawAddress) & 0x1fffff
                : (base + this.rawAddress) & 0x1fffff
        else this.address = undefined
      }
      const address = this.address
      if (address !== undefined)
        this.address =
          this.bytes === 4 ? address & 0x1ffffc : this.bytes === 2 ? address & 0x1ffffe : address
      await this.hostHeader(at)
      return
    }
    if (this.phase === 'host-data') {
      await this.dataBit(bit, at, true)
      return
    }
    if (this.phase === 'crc-host') {
      this.received = (this.received << 1) | bit
      this.bits += 1
      if (this.bits !== 4) return
      this.hostOk = this.crc === 0
      await this.out.emit(
        {
          kind: 'crc',
          packet: this.packet,
          host: true,
          received: this.received,
          expected: this.expected,
          ok: this.hostOk,
        },
        at,
      )
      if (
        this.hostOk &&
        (this.command === 'absolute' || this.command === 'relative' || this.command === 'same')
      ) {
        this.pending = true
        this.pendingExchange = this.exchange
        this.pendingSlave = this.slave
        this.pendingRead = this.read
        this.pendingBytes = this.bytes
        this.pendingAddress = this.address
      }
      if (!this.hostOk) {
        this.setCache(this.slave, undefined)
        this.pending = false
      }
      this.first = undefined
      this.tar = this.config.turnaround
      this.phase = 'tar-response'
      this.waited = 0
    }
  }
  async slaveBit(bit: u8, at: Provenance): Promise<void> {
    if (this.phase === 'response-wait') {
      if (bit === 0) {
        this.waited += 1
        if (this.waited === this.config.responseTimeout) {
          await this.end('timeout', false, at)
          this.phase = 'unsynced'
          this.forget()
        }
        return
      }
      this.first = at
      this.phase = 'slave-id'
      this.bits = 0
      this.value = 0
      this.crc = crc4(0, 1)
      return
    }
    this.crc = crc4(this.crc, bit)
    if (this.phase === 'slave-id') {
      this.value = (this.value << 1) | bit
      this.bits += 1
      if (this.bits === 2) {
        this.responseSlave = this.value as u8
        this.responseRelated =
          this.responseSlave === this.slave &&
          this.hostOk &&
          (this.command !== 'data-poll' || this.related)
        this.phase = 'status'
        this.bits = 0
        this.value = 0
      }
      return
    }
    if (this.phase === 'status') {
      this.value = (this.value << 1) | bit
      this.bits += 1
      const interrupt = this.command === 'interrupt-poll' && this.bits === 1 && this.value === 0
      if (!interrupt && this.bits !== 2) return
      this.status = interrupt
        ? 'interrupt'
        : this.value === 0
          ? 'ack'
          : this.value === 1
            ? 'busy'
            : this.value === 2
              ? 'error-a'
              : 'error-crc'
      const first = this.first
      const placed = first === undefined ? at : join(first, at)
      await this.out.emit(
        {
          kind: 'response',
          packet: this.packet,
          exchange: this.exchange,
          slave: this.responseSlave,
          status: this.status,
          related: this.responseRelated,
        },
        placed,
      )
      if (interrupt) {
        this.phase = 'interrupt'
        this.bits = 0
        this.value = 0
      } else if (this.status === 'ack' && this.command === 'data-poll' && !this.related) {
        await this.end('unknown-context', false, at)
        this.phase = 'unsynced'
      } else if (this.status === 'ack' && this.read && this.bytes !== 0) {
        this.phase = 'slave-data'
        this.byteIndex = 0
        this.byteBits = 0
        this.byteValue = 0
      } else this.startCrc(false)
      return
    }
    if (this.phase === 'slave-data') {
      await this.dataBit(bit, at, false)
      return
    }
    if (this.phase === 'interrupt') {
      this.value = (this.value << 1) | bit
      this.bits += 1
      if (this.bits === 5) {
        await this.out.emit(
          {
            kind: 'interrupt',
            packet: this.packet,
            // eslint-disable-next-line @typescript-eslint/no-unnecessary-type-assertion -- The DSL IR requires an explicit u8 narrowing at this boundary.
            levels: (this.value >> 3) as u8,
            // eslint-disable-next-line @typescript-eslint/no-unnecessary-type-assertion -- The DSL IR requires an explicit u8 narrowing at this boundary.
            dma: (this.value & 7) as u8,
          },
          at,
        )
        this.startCrc(false)
      }
      return
    }
    if (this.phase === 'crc-slave') {
      this.received = (this.received << 1) | bit
      this.bits += 1
      if (this.bits !== 4) return
      const ok = this.crc === 0
      await this.out.emit(
        {
          kind: 'crc',
          packet: this.packet,
          host: false,
          received: this.received,
          expected: this.expected,
          ok,
        },
        at,
      )
      const success =
        ok && this.responseRelated && (this.status === 'ack' || this.status === 'interrupt')
      if (success && this.pending && this.pendingSlave === this.responseSlave) {
        const address = this.pendingAddress
        this.setCache(this.pendingSlave, address === undefined ? undefined : address & 0x1ffffc)
        this.pending = false
      }
      if (
        !ok ||
        !this.responseRelated ||
        this.status === 'error-a' ||
        this.status === 'error-crc'
      ) {
        this.setCache(this.slave, undefined)
        this.pending = false
      }
      await this.end(
        !ok || !this.hostOk
          ? 'bad-crc'
          : !this.responseRelated
            ? 'invalid'
            : this.status === 'busy'
              ? 'busy'
              : this.status === 'ack' || this.status === 'interrupt'
                ? 'ack'
                : 'error',
        success,
        at,
      )
    }
  }
  override async run(): Promise<void> {
    await this.signal.sample()
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      const rising = edge.levels.high(this.config.clock)
      const bit: u8 = fromBool(edge.levels.high(this.config.data) !== this.config.inverted)
      const at = one(edge.tick)
      if (rising) {
        if (bit === 1) {
          if (this.breakRun === 0) this.breakFirst = edge.tick
          if (this.breakRun < 257) this.breakRun += 1
          if (this.breakRun === 256) {
            const first = this.breakFirst
            await this.cut(edge.tick)
            await this.out.emit(
              { kind: 'break', cycles: 256 },
              first === undefined ? at : over(first, edge.tick + 1, edge.tick),
            )
            this.breakFirst = undefined
            this.breaking = true
            continue
          }
        } else {
          this.breakRun = 0
          this.breakFirst = undefined
          if (this.breaking) {
            this.breaking = false
            this.phase = 'tar-command'
            this.tar = this.config.turnaround
            continue
          }
        }
      }
      if (this.breaking) continue
      if (this.phase === 'tar-response') {
        if (rising) {
          this.tar -= 1
          if (this.tar === 0) this.phase = 'response-wait'
        }
        continue
      }
      if (this.phase === 'tar-command') {
        if (!rising) {
          this.tar -= 1
          if (this.tar === 0) this.phase = 'idle'
        }
        continue
      }
      if (this.phase === 'unsynced') continue
      if (
        this.phase === 'response-wait' ||
        this.phase === 'slave-id' ||
        this.phase === 'status' ||
        this.phase === 'slave-data' ||
        this.phase === 'interrupt' ||
        this.phase === 'crc-slave'
      ) {
        if (!rising) await this.slaveBit(bit, at)
      } else if (rising) await this.hostBit(bit, at)
    }
  }
}
export const fsi = defineLogicDecoder({
  name: 'fsi',
  category: 'bus',
  config: {
    defaults: {
      clock: 0,
      data: 1,
      inverted: true,
      turnaround: 3,
      responseTimeout: 256,
      startAligned: false,
    },
    validate,
  },
  requires: (config) => ({ channels: atLeast(max(config.clock, config.data) + 1) }),
  output: events,
  props: () => ({}),
  locales: {
    'en-US': {
      name: 'OpenFSI',
      summary: 'CRC-checked address commands, busy polling and response context',
      fields: {
        clock: 'FSI clock',
        data: 'FSI data',
        inverted: 'Electrically inverted data',
        turnaround: 'Turnaround cycles',
        responseTimeout: 'Response timeout cycles',
        startAligned: 'Start at a known command boundary',
      },
      events: {
        break: 'BREAK',
        command: 'Command',
        response: 'Response',
        byte: 'Data byte',
        interrupt: 'Interrupt / DMA',
        crc: 'CRC4',
        end: 'Packet end',
      },
      eventFields: {
        cycles: 'Cycles',
        packet: 'Packet',
        exchange: 'Exchange',
        slave: 'Slave ID',
        command: 'Command',
        read: 'Read',
        bytes: 'Data bytes',
        address: 'Known byte address',
        related: 'Known association',
        status: 'Response status',
        index: 'Byte index',
        value: 'Value',
        levels: 'Interrupt levels',
        dma: 'DMA control',
        host: 'Host command',
        received: 'Received CRC',
        expected: 'Expected CRC',
        ok: 'CRC OK',
        reason: 'End reason',
        success: 'Valid acknowledgement',
      },
    },
    'zh-CN': {
      name: 'OpenFSI',
      summary: '地址命令CRC、忙状态轮询及响应关联',
      fields: {
        clock: 'FSI时钟',
        data: 'FSI数据',
        inverted: '数据电平反相',
        turnaround: '换向周期',
        responseTimeout: '响应超时周期',
        startAligned: '从已知命令边界开始',
      },
      events: {
        break: 'BREAK',
        command: '命令',
        response: '响应',
        byte: '数据字节',
        interrupt: '中断/DMA',
        crc: 'CRC4',
        end: '数据包结束',
      },
      eventFields: {
        cycles: '周期数',
        packet: '数据包',
        exchange: '交互编号',
        slave: '从机ID',
        command: '命令',
        read: '读取',
        bytes: '数据字节数',
        address: '已知字节地址',
        related: '关联已知',
        status: '响应状态',
        index: '字节索引',
        value: '数值',
        levels: '中断电平',
        dma: 'DMA控制',
        host: '主机命令',
        received: '接收CRC',
        expected: '预期CRC',
        ok: 'CRC正确',
        reason: '结束原因',
        success: '有效确认',
      },
    },
  },
  decoder: Fsi,
})
