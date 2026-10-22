// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Integer assertions select IR widths and truncation. */
/** ADIv5 SW-DP register observations and posted AP reads, Arm IHI0031A §§5,6,8.
 * ACK admits an AP request; it does not prove the later memory operation succeeded. */
import type { bool, u8, u32, u64, Tick, Range, Provenance } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, join } from 'uiharu'
import { swdEvents, type SwdEvent, type Ack, type EndReason } from '../contracts/swd'
export interface SwdDpConfig {
  ap: u8
  tarWrapBits: u8
}
export type Register =
  | 'dpidr'
  | 'abort'
  | 'ctrl-stat'
  | 'select'
  | 'resend'
  | 'rdbuff'
  | 'targetsel'
  | 'dp-register'
  | 'csw'
  | 'tar'
  | 'drw'
  | 'ap-register'
export type Event =
  | {
      kind: 'access'
      tx: u64
      ap: bool
      apIndex: u8 | undefined
      address: u8 | undefined
      register: Register
      read: bool
      value: u32 | undefined
      ack: Ack
      reason: EndReason
    }
  | { kind: 'posted-result'; tx: u64; responseTx: u64; register: Register; value: u32 }
  | {
      kind: 'resend-result'
      tx: u64
      originalTx: u64 | undefined
      value: u32
      matchesPrevious: bool | undefined
    }
  | {
      kind: 'memory-request'
      tx: u64
      apIndex: u8
      plannedAddress: u32 | undefined
      sizeBits: u8 | undefined
      packed: bool
      read: bool
      rawData: u32 | undefined
    }
  | {
      kind: 'memory-result'
      tx: u64
      responseTx: u64
      plannedAddress: u32 | undefined
      rawData: u32
    }
  | { kind: 'status'; tx: u64; value: u32; stickyFault: bool }
  | { kind: 'context-lost' }
export const events = eventSchema<Event>('swd-dp.events', 1).roles({
  access: 'command',
  'posted-result': 'data',
  'memory-request': 'address',
  'memory-result': 'data',
  'resend-result': 'data',
  status: { role: 'control', errorWhen: { stickyFault: true } },
  'context-lost': 'line',
})
interface Pending {
  tx: u64
  register: Register
  selected: bool
  planned: u32 | undefined
}
function validate(c: SwdDpConfig): string | undefined {
  if (c.tarWrapBits < 10 || c.tarWrapBits > 31) return 'decode.config.tarWrapBits'
  return undefined
}
class SwdDp extends StreamDecoder<SwdDpConfig, SwdEvent, Event> {
  packetTx: u64 | undefined = undefined
  first: Provenance | undefined = undefined
  ap = false
  read = false
  addr: u8 = 0
  valid = false
  ack: Ack = { other: 0 }
  data: u32 | undefined = undefined
  select: u32 | undefined = undefined
  csw: u32 | undefined = undefined
  tar: u32 | undefined = undefined
  pending: Pending | undefined = undefined
  lastReadTx: u64 | undefined = undefined
  lastReadValue: u32 | undefined = undefined
  override hold(): Tick | undefined {
    const first = this.first
    return first?.span.start
  }
  clear(): void {
    this.select = undefined
    this.csw = undefined
    this.tar = undefined
    this.pending = undefined
    this.lastReadTx = undefined
    this.lastReadValue = undefined
  }
  override onGap(_range: Range): void {
    this.clear()
    this.packetTx = undefined
    this.first = undefined
  }
  override onEnd(_tick: Tick): void {
    this.packetTx = undefined
    this.first = undefined
    this.clear()
  }
  name(bank: u8 | undefined): Register {
    if (this.ap) {
      if (bank === 0) {
        if (this.addr === 0) return 'csw'
        if (this.addr === 4) return 'tar'
        if (this.addr === 12) return 'drw'
      }
      return 'ap-register'
    }
    if (this.addr === 0) return this.read ? 'dpidr' : 'abort'
    if (this.addr === 8) return this.read ? 'resend' : 'select'
    if (this.addr === 12) return this.read ? 'rdbuff' : 'targetsel'
    if (bank === 0) return 'ctrl-stat'
    return 'dp-register'
  }
  advance(): void {
    const csw = this.csw
    const tar = this.tar
    if (tar === undefined) return
    if (csw === undefined) {
      this.tar = undefined
      return
    }
    const size: u8 = (csw & 7) as u8
    const inc: u8 = ((csw >> 4) & 3) as u8
    if (size > 2 || inc === 3) {
      this.tar = undefined
      return
    }
    if (inc === 0) return
    const stride: u32 = inc === 2 ? 4 : 1 << size
    const mask: u32 = (1 << this.config.tarWrapBits) - 1
    this.tar = (tar & ~mask) | ((tar + stride) & mask)
  }
  async response(responseTx: u64, value: u32, at: Provenance): Promise<void> {
    const pending = this.pending
    if (pending === undefined) {
      this.lastReadTx = undefined
      this.lastReadValue = value
      return
    }
    await this.out.emit(
      { kind: 'posted-result', tx: pending.tx, responseTx, register: pending.register, value },
      at,
    )
    if (pending.selected) {
      if (pending.register === 'csw') this.csw = value
      if (pending.register === 'tar') this.tar = value
      if (pending.register === 'drw')
        await this.out.emit(
          {
            kind: 'memory-result',
            tx: pending.tx,
            responseTx,
            plannedAddress: pending.planned,
            rawData: value,
          },
          at,
        )
    }
    this.lastReadTx = pending.tx
    this.lastReadValue = value
    this.pending = undefined
  }
  async finish(tx: u64, reason: EndReason, at: Provenance): Promise<void> {
    const select = this.select
    const apIndex: u8 | undefined = select === undefined ? undefined : ((select >> 24) as u8)
    const bank: u8 | undefined =
      select === undefined
        ? undefined
        : this.ap
          ? (((select >> 4) & 15) as u8)
          : ((select & 15) as u8)
    const address: u8 | undefined =
      bank === undefined ? undefined : (((bank << 4) | this.addr) as u8)
    const register = this.name(bank)
    const observedValue: u32 = this.data ?? 0
    await this.out.emit(
      {
        kind: 'access',
        tx,
        ap: this.ap,
        apIndex: this.ap ? apIndex : undefined,
        address: this.ap ? address : this.addr,
        register,
        read: this.read,
        value: this.data,
        ack: this.ack,
        reason,
      },
      at,
    )
    if (reason === 'wait' && this.valid && this.ack === 'wait') return
    if (!this.valid || reason !== 'complete' || this.ack !== 'ok' || this.data === undefined) {
      this.clear()
      await this.out.emit({ kind: 'context-lost' }, at)
      return
    }
    // AP read data belongs to the previous accepted AP read, even if APSEL changes.
    if ((this.ap && this.read) || register === 'rdbuff') await this.response(tx, observedValue, at)
    if (!this.ap) {
      if (register === 'resend') {
        const lastReadValue = this.lastReadValue
        await this.out.emit(
          {
            kind: 'resend-result',
            tx,
            originalTx: this.lastReadTx,
            value: observedValue,
            matchesPrevious:
              lastReadValue === undefined ? undefined : lastReadValue === observedValue,
          },
          at,
        )
      }
      if (register === 'select') this.select = observedValue
      if (register === 'abort' || register === 'targetsel') {
        this.clear()
        await this.out.emit({ kind: 'context-lost' }, at)
      }
      if (register === 'ctrl-stat' && this.read) {
        const stickyFault = (observedValue & 0xa2) !== 0
        await this.out.emit({ kind: 'status', tx, value: observedValue, stickyFault }, at)
        if (stickyFault) {
          this.csw = undefined
          this.tar = undefined
          this.pending = undefined
        }
      }
      return
    }
    const selected = apIndex === this.config.ap
    const planned = selected ? this.tar : undefined
    if (selected && register === 'drw') {
      const csw = this.csw
      const size: u8 | undefined = csw === undefined ? undefined : ((csw & 7) as u8)
      await this.out.emit(
        {
          kind: 'memory-request',
          tx,
          apIndex: this.config.ap,
          plannedAddress: planned,
          sizeBits: size === undefined ? undefined : size <= 2 ? ((8 << size) as u8) : undefined,
          packed: csw === undefined ? false : ((csw >> 4) & 3) === 2,
          read: this.read,
          rawData: this.read ? undefined : observedValue,
        },
        at,
      )
      this.advance()
    }
    if (this.read) this.pending = { tx, register, selected, planned }
    else if (selected) {
      if (register === 'csw') this.csw = observedValue
      if (register === 'tar') this.tar = observedValue
    }
  }
  override async run(): Promise<void> {
    for (;;) {
      const item = await this.input.next()
      const event = item.event
      if (event.kind === 'line-reset' || event.kind === 'switch') {
        this.clear()
        this.packetTx = undefined
        this.first = undefined
        await this.out.emit({ kind: 'context-lost' }, item.at)
        continue
      }
      if (event.kind === 'begin') {
        this.packetTx = event.tx
        this.first = item.at
        this.valid = false
        this.ack = { other: 0 }
        this.data = undefined
        continue
      }
      if (event.kind === 'request') {
        if (this.packetTx !== event.tx) continue
        this.ap = event.apndp
        this.read = event.rnw
        this.addr = event.addr
        this.valid = event.parityOk
      }
      if (event.kind === 'ack' && this.packetTx === event.tx) this.ack = event.value
      if (event.kind === 'data') {
        if (this.packetTx !== event.tx) continue
        this.data = event.value
        this.valid = this.valid && event.parityOk && event.fromTarget === this.read
      }
      if (event.kind === 'end') {
        if (this.packetTx !== event.tx) continue
        const first = this.first
        await this.finish(
          event.tx,
          event.reason,
          first === undefined ? item.at : join(first, item.at),
        )
        this.packetTx = undefined
        this.first = undefined
      }
    }
  }
}
export const swdDp = defineStreamDecoder({
  name: 'swd-dp',
  category: 'device',
  config: { defaults: { ap: 0, tarWrapBits: 10 }, validate },
  input: swdEvents,
  output: events,
  props: () => ({}),
  locales: {
    'en-US': {
      name: 'Arm SWD DP/AP',
      summary: 'SW-DP registers, posted AP read results and planned MEM-AP addresses',
      fields: { ap: 'MEM-AP index', tarWrapBits: 'TAR increment wrap bits' },
      events: {
        access: 'DP/AP access',
        'posted-result': 'Posted read result',
        'resend-result': 'Repeated read result',
        'memory-request': 'Planned memory access',
        'memory-result': 'Observed memory read result',
        status: 'DP status',
        'context-lost': 'Context lost',
      },
      eventFields: {
        tx: 'Transfer',
        ap: 'Access port',
        apIndex: 'AP index',
        address: 'Register address',
        register: 'Register',
        read: 'Read',
        value: 'Value',
        ack: 'Acknowledge',
        reason: 'End reason',
        responseTx: 'Response transfer',
        originalTx: 'Original read transfer',
        matchesPrevious: 'Matches previous result',
        plannedAddress: 'Planned address',
        sizeBits: 'Transfer bits',
        packed: 'Packed increment',
        rawData: 'Raw data',
        stickyFault: 'Sticky fault',
      },
    },
    'zh-CN': {
      name: 'Arm SWD DP/AP',
      summary: 'SW-DP寄存器、AP延迟读取结果与MEM-AP候选地址',
      fields: { ap: 'MEM-AP索引', tarWrapBits: 'TAR自增回绕位数' },
      events: {
        access: 'DP/AP访问',
        'posted-result': '延迟读取结果',
        'resend-result': '重发读取结果',
        'memory-request': '候选内存访问',
        'memory-result': '观测内存读取结果',
        status: 'DP状态',
        'context-lost': '上下文丢失',
      },
      eventFields: {
        tx: '传输',
        ap: '访问端口',
        apIndex: 'AP索引',
        address: '寄存器地址',
        register: '寄存器',
        read: '读取',
        value: '数值',
        ack: '应答',
        reason: '结束原因',
        responseTx: '响应传输',
        originalTx: '原始读取传输',
        matchesPrevious: '与上次结果一致',
        plannedAddress: '候选地址',
        sizeBits: '传输位数',
        packed: '打包自增',
        rawData: '原始数据',
        stickyFault: '粘滞错误',
      },
    },
  },
  decoder: SwdDp,
})
