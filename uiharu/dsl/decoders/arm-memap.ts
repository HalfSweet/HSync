// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Integer assertions select IR widths and truncation; TypeScript aliases alone do not encode these semantics. */

/**
 * ADIv5 DP/AP register interpretation and MEM-AP planned accesses over JTAG-DP.
 * Primary reference ARM IHI0031A §§4.4,6,8,11:
 * https://documentation-service.arm.com/static/5f900a61f86e16515cdc0610?token=
 * JTAG's OK/FAULT acknowledgement cannot establish success. Events therefore distinguish
 * admitted requests, captured prior responses and planned addresses from proven memory data.
 */
import type { Provenance, Range, Tick, bool, u32, u64, u8 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema } from 'uiharu'
import { jtagDpEvents, type JtagDpEvent } from '../contracts/jtag-dp'

export interface ArmMemapConfig {
  ap: u8
  tarWrapBits: u8
}
export type RegisterName =
  'select' | 'ctrl-stat' | 'rdbuff' | 'dp-register' | 'csw' | 'tar' | 'drw' | 'ap-register'
export type ArmMemapEvent =
  | {
      kind: 'request'
      scan: u64
      ap: bool
      apIndex: u8 | undefined
      register: RegisterName
      address: u8 | undefined
      read: bool
      value: u32
    }
  | {
      kind: 'response'
      scan: u64
      responseScan: u64
      value: u32
      register: RegisterName
      okOrFault: bool
    }
  | {
      kind: 'memory-request'
      scan: u64
      apIndex: u8
      plannedAddress: u32 | undefined
      sizeBits: u8 | undefined
      packed: bool
      read: bool
      rawData: u32
    }
  | {
      kind: 'memory-response'
      scan: u64
      responseScan: u64
      plannedAddress: u32 | undefined
      rawData: u32
      okOrFault: bool
    }
  | { kind: 'status'; scan: u64; value: u32; stickyFault: bool }
  | { kind: 'wait'; scan: u64 }
  | { kind: 'context-lost' }
export const armMemapEvents = eventSchema<ArmMemapEvent>('arm-memap.events', 1).roles({
  request: 'command',
  response: 'data',
  'memory-request': 'address',
  'memory-response': 'data',
  status: { role: 'control', errorWhen: { stickyFault: true } },
  wait: 'handshake',
  'context-lost': 'line',
})
interface Pending {
  scan: u64
  ap: bool
  read: bool
  addr: u8
  value: u32
  register: RegisterName
  selected: bool
  planned: u32 | undefined
}
function validate(config: ArmMemapConfig): string | undefined {
  if (config.tarWrapBits < 10 || config.tarWrapBits > 31) return 'decode.config.tarWrapBits'
  return undefined
}
class ArmMemap extends StreamDecoder<ArmMemapConfig, JtagDpEvent, ArmMemapEvent> {
  select: u32 | undefined = undefined
  csw: u32 | undefined = undefined
  tar: u32 | undefined = undefined
  pending: Pending | undefined = undefined
  clear(): void {
    this.select = undefined
    this.csw = undefined
    this.tar = undefined
    this.pending = undefined
  }
  override onGap(_range: Range): void {
    this.clear()
  }
  override onEnd(_tick: Tick): void {
    this.clear()
  }

  name(ap: bool, addr: u8, bank: u8 | undefined): RegisterName {
    if (ap) {
      if (bank === 0) {
        if (addr === 0) return 'csw'
        if (addr === 4) return 'tar'
        if (addr === 12) return 'drw'
      }
      return 'ap-register'
    }
    if (addr === 8) return 'select'
    if (addr === 4 && bank === 0) return 'ctrl-stat'
    if (addr === 12) return 'rdbuff'
    return 'dp-register'
  }
  nextTar(): void {
    const tar = this.tar
    const csw = this.csw
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
  async completed(pending: Pending, responseScan: u64, value: u32, at: Provenance): Promise<void> {
    await this.out.emit(
      {
        kind: 'response',
        scan: pending.scan,
        responseScan,
        value,
        register: pending.register,
        okOrFault: true,
      },
      at,
    )
    if (pending.register === 'ctrl-stat' && pending.read) {
      const stickyFault = (value & 0xa2) !== 0
      await this.out.emit({ kind: 'status', scan: pending.scan, value, stickyFault }, at)
      if (stickyFault) {
        this.csw = undefined
        this.tar = undefined
      }
    }
    // A SELECT write is a DP-local operation; candidate AP state still remains explicitly
    // planned because AP writes and captured results share the ambiguous OK/FAULT response.
    if (pending.register === 'select' && !pending.read) this.select = pending.value
    if (pending.selected) {
      if (pending.register === 'csw') this.csw = pending.read ? value : pending.value
      if (pending.register === 'tar') this.tar = pending.read ? value : pending.value
      if (pending.register === 'drw') {
        await this.out.emit(
          {
            kind: 'memory-response',
            scan: pending.scan,
            responseScan,
            plannedAddress: pending.planned,
            rawData: value,
            okOrFault: true,
          },
          at,
        )
        this.nextTar()
      }
    }
  }
  override async run(): Promise<void> {
    for (;;) {
      const received = await this.input.next()
      const event = received.event
      const at = received.at
      if (event.kind === 'reset' || event.kind === 'abort') {
        this.clear()
        await this.out.emit({ kind: 'context-lost' }, at)
      } else if (
        event.kind === 'instruction' &&
        event.value !== 10 &&
        event.value !== 11 &&
        event.value !== 15
      ) {
        this.clear()
        await this.out.emit({ kind: 'context-lost' }, at)
      } else if (event.kind === 'scan') {
        if (event.status === 'wait') await this.out.emit({ kind: 'wait', scan: event.scan }, at)
        else {
          this.clear()
          await this.out.emit({ kind: 'context-lost' }, at)
        }
      } else if (event.kind === 'access') {
        if (event.ack === 'wait') {
          await this.out.emit({ kind: 'wait', scan: event.scan }, at)
          continue
        }
        if (event.ack !== 'ok-fault' || event.accepted !== true) {
          this.clear()
          await this.out.emit({ kind: 'context-lost' }, at)
          continue
        }
        const previous = this.pending
        if (previous !== undefined) {
          if (event.previousScan === previous.scan)
            await this.completed(previous, event.scan, event.responseData, at)
          else {
            this.select = undefined
            this.csw = undefined
            this.tar = undefined
          }
        } else if (event.previousScan !== undefined) {
          this.select = undefined
          this.csw = undefined
          this.tar = undefined
        }
        this.pending = undefined
        const ap = event.instruction === 11
        const select = this.select
        let apIndex: u8 | undefined = undefined
        let bank: u8 | undefined = undefined
        if (select !== undefined) {
          apIndex = ((select >> 24) & 0xff) as u8
          bank = (ap ? (select >> 4) & 0xf : select & 0xf) as u8
        }
        const name = this.name(ap, event.addr, bank)
        let address: u8 | undefined = event.addr
        if (ap) {
          if (bank === undefined) address = undefined
          else address = ((bank << 4) | event.addr) as u8
        }
        const selected = ap && apIndex === this.config.ap
        const planned = selected && name === 'drw' ? this.tar : undefined
        await this.out.emit(
          {
            kind: 'request',
            scan: event.scan,
            ap,
            apIndex,
            register: name,
            address,
            read: event.read,
            value: event.dataIn,
          },
          at,
        )
        if (selected && name === 'drw') {
          const csw = this.csw
          let sizeBits: u8 | undefined = undefined
          let packed = false
          if (csw !== undefined) {
            const size: u8 = (csw & 7) as u8
            const baseSize: u8 = 8
            if (size <= 2) sizeBits = baseSize << size
            packed = ((csw >> 4) & 3) === 2
          }
          await this.out.emit(
            {
              kind: 'memory-request',
              scan: event.scan,
              apIndex: this.config.ap,
              plannedAddress: planned,
              sizeBits,
              packed,
              read: event.read,
              rawData: event.dataIn,
            },
            at,
          )
        }
        this.pending = {
          scan: event.scan,
          ap,
          read: event.read,
          addr: event.addr,
          value: event.dataIn,
          register: name,
          selected,
          planned,
        }
      }
    }
  }
}
export const armMemap = defineStreamDecoder({
  name: 'arm-memap',
  category: 'device',
  config: { defaults: { ap: 0, tarWrapBits: 10 }, validate },
  input: jtagDpEvents,
  output: armMemapEvents,
  locales: {
    'en-US': {
      name: 'Arm MEM-AP',
      summary: 'DP/AP requests and planned memory addresses; OK/FAULT remains ambiguous',
      fields: { ap: 'Access port index', tarWrapBits: 'TAR auto-increment wrap bits' },
      events: {
        request: 'Register request',
        response: 'Captured response',
        'memory-request': 'Planned memory access',
        'memory-response': 'Captured memory response',
        status: 'Debug status',
        wait: 'WAIT',
        'context-lost': 'Context lost',
      },
      eventFields: {
        scan: 'Request scan',
        responseScan: 'Response scan',
        ap: 'Access port',
        apIndex: 'AP index',
        register: 'Register',
        address: 'Register address',
        read: 'Read',
        value: 'Value',
        okOrFault: 'OK or FAULT',
        plannedAddress: 'Planned address',
        sizeBits: 'Access size (bits)',
        packed: 'Packed',
        rawData: 'Raw data',
        stickyFault: 'Sticky fault',
      },
    },
    'zh-CN': {
      name: 'Arm MEM-AP',
      summary: 'DP/AP 请求及计划访问地址；OK/FAULT 仍无法区分',
      fields: { ap: '访问端口编号', tarWrapBits: 'TAR 自动递增回绕位数' },
      events: {
        request: '寄存器请求',
        response: '捕获的响应',
        'memory-request': '计划内存访问',
        'memory-response': '捕获的内存响应',
        status: '调试状态',
        wait: 'WAIT',
        'context-lost': '上下文丢失',
      },
      eventFields: {
        scan: '请求扫描',
        responseScan: '响应扫描',
        ap: '访问端口',
        apIndex: 'AP 编号',
        register: '寄存器',
        address: '寄存器地址',
        read: '读取',
        value: '值',
        okOrFault: 'OK 或 FAULT',
        plannedAddress: '计划地址',
        sizeBits: '访问宽度（位）',
        packed: '打包传输',
        rawData: '原始数据',
        stickyFault: '粘滞故障',
      },
    },
  },
  decoder: ArmMemap,
})
