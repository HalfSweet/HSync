// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Integer assertions select IR widths and truncation; TypeScript aliases alone do not encode these semantics. */
/** Independently authored from Arm IHI0014Q Chapter7, packet/atom/branch diagrams.
 * https://documentation-service.arm.com/static/5f90158b4966cd7c95fd5b5e */
import type { bool, u8, u16, u32, u64, Provenance, Tick, Range } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
import { traceByteEvents, type TraceByteEvent } from './arm-trace-transport'
export type BranchEncoding = 'original' | 'alternative'
export type Isa = 'arm' | 'thumb' | 'jazelle'
export type Atom = 'e' | 'n' | 'w'
export type Marker =
  'trigger' | 'store-failed' | 'data-suppressed' | 'ignore' | 'exception-entry' | 'exception-exit'
export type ErrorReason =
  'aborted' | 'input-error' | 'unsupported-header' | 'malformed' | 'overlong' | 'bad-sync'
export interface ArmEtmv3Config {
  startAligned: bool
  contextBytes: u8
  cycleAccurate: bool
  minorVersion: u8
  branchEncoding: BranchEncoding
  dataAddresses: bool
  timestampBits: u8
}
export type ArmEtmv3Event =
  | { kind: 'sync' }
  | { kind: 'atom'; atom: Atom; index: u8; header: u8 }
  | { kind: 'marker'; marker: Marker }
  | { kind: 'cycle-count'; value: u32; overflow: bool }
  | {
      kind: 'isync'
      address: u32
      context: u32 | undefined
      reason: u8
      isa: Isa
      altIsa: bool
      nonsecure: bool
      hyp: bool
      loadStoreInProgress: bool
    }
  | {
      kind: 'branch'
      address: u32 | undefined
      fragment: u32
      significantBits: u8
      isa: Isa | undefined
      exception: u16
      legacyException: u8 | undefined
      cancel: bool
      resume: u8
      nonsecure: bool | undefined
      hyp: bool | undefined
      altIsa: bool | undefined
    }
  | { kind: 'context'; value: u32 }
  | { kind: 'vmid'; value: u8 }
  | {
      kind: 'data'
      tag: u8
      placeholder: bool
      valueTraced: bool
      value: u32 | undefined
      bytes: u8
      address: u32 | undefined
      bigEndian: bool | undefined
    }
  | { kind: 'timestamp'; fragment: u64; significantBits: u8; clockChange: bool }
  | { kind: 'error'; header: u8 | undefined; reason: ErrorReason }
export const armEtmv3Events = eventSchema<ArmEtmv3Event>('arm-etmv3.events', 1).roles({
  sync: 'framing',
  atom: 'control',
  marker: 'control',
  'cycle-count': 'control',
  isync: 'address',
  branch: 'address',
  context: 'control',
  vmid: 'control',
  data: 'data',
  timestamp: 'control',
  error: 'error',
})
function validate(c: ArmEtmv3Config): string | undefined {
  if (c.contextBytes !== 0 && c.contextBytes !== 1 && c.contextBytes !== 2 && c.contextBytes !== 4)
    return 'decode.config.etmContextBytes'
  if (c.minorVersion > 5) return 'decode.config.etmVersion'
  if (c.timestampBits !== 48 && c.timestampBits !== 64) return 'decode.config.etmTimestampBits'
  return undefined
}
class ArmEtmv3 extends StreamDecoder<ArmEtmv3Config, TraceByteEvent, ArmEtmv3Event> {
  aligned = this.config.startAligned
  mode: u8 = 0
  header: u8 = 0
  first: Provenance | undefined = undefined
  zeros: u8 = 0
  zeroAt: Provenance | undefined = undefined
  count: u8 = 0
  shift: u8 = 0
  value: u64 = 0
  info: u8 = 0
  context: u32 | undefined = undefined
  address: u32 | undefined = undefined
  isa: Isa | undefined = undefined
  nonsecure: bool | undefined = undefined
  hyp: bool | undefined = undefined
  altIsa: bool | undefined = undefined
  branch: u32 | undefined = undefined
  branchFragment: u32 = 0
  branchBits: u8 = 0
  legacyException: u8 | undefined = undefined
  exception: u16 = 0
  cancel = false
  resume: u8 = 0
  excCount: u8 = 0
  dataAddress: u32 | undefined = undefined
  dataCurrent: u32 | undefined = undefined
  bigEndian: bool | undefined = undefined
  dataBytes: u8 = 0
  dataTag: u8 = 0
  placeholder = false
  valueTraced = true
  lsip = false
  override hold(): Tick | undefined {
    const f = this.first
    if (f !== undefined) return f.span.start
    const z = this.zeroAt
    if (z !== undefined) return z.span.start
    return undefined
  }
  reset(): void {
    this.mode = 0
    this.first = undefined
    this.zeros = 0
    this.zeroAt = undefined
    this.context = undefined
    this.address = undefined
    this.isa = undefined
    this.nonsecure = undefined
    this.hyp = undefined
    this.altIsa = undefined
    this.dataAddress = undefined
    this.bigEndian = undefined
  }
  async fail(reason: ErrorReason, at: Provenance): Promise<void> {
    const f = this.first
    await this.out.emit(
      { kind: 'error', header: this.mode === 0 ? undefined : this.header, reason },
      f === undefined ? at : over(f.span.start, at.span.end, at.settled),
    )
    this.reset()
    this.aligned = false
  }
  override async onGap(range: Range): Promise<void> {
    if (this.mode !== 0) await this.fail('aborted', over(range.start, range.start, range.start))
    this.reset()
    this.aligned = false
  }
  override async onEnd(tick: Tick): Promise<void> {
    if (this.mode !== 0) await this.fail('aborted', over(tick, tick, tick))
    this.reset()
  }
  begin(mode: u8, header: u8, at: Provenance): void {
    this.mode = mode
    this.header = header
    this.first = at
    this.count = 0
    this.shift = 0
    this.value = 0
  }
  placed(at: Provenance): Provenance {
    const f = this.first
    return f === undefined ? at : over(f.span.start, at.span.end, at.settled)
  }
  done(): void {
    this.mode = 0
    this.first = undefined
  }
  async atom(atom: Atom, index: u8, header: u8, at: Provenance): Promise<void> {
    await this.out.emit({ kind: 'atom', atom, index, header }, at)
  }
  async atoms(byte: u8, at: Provenance): Promise<void> {
    let index: u8 = 0
    if ((byte & 3) === 0) {
      if (this.config.cycleAccurate && (byte & 0x20) !== 0) {
        const waits: u8 = ((byte >> 2) & 7) + 1
        for (let i: u8 = 0; i < waits; i += 1) {
          await this.atom('w', index, byte, at)
          index += 1
        }
        if ((byte & 0x40) !== 0) await this.atom('e', index, byte, at)
        return
      }
      const executed: u8 = (byte >> 2) & (this.config.cycleAccurate ? 7 : 15)
      if (this.config.cycleAccurate && byte === 0x80) {
        if (this.config.minorVersion === 0) await this.atom('w', 0, byte, at)
        else await this.fail('malformed', at)
        return
      }
      for (let i: u8 = 0; i < executed; i += 1) {
        if (this.config.cycleAccurate) {
          await this.atom('w', index, byte, at)
          index += 1
        }
        await this.atom('e', index, byte, at)
        index += 1
      }
      if ((byte & 0x40) !== 0) {
        if (this.config.cycleAccurate) {
          await this.atom('w', index, byte, at)
          index += 1
        }
        await this.atom('n', index, byte, at)
      }
      return
    }
    if ((byte & 0xf3) === 0x82) {
      if (this.config.cycleAccurate) {
        await this.atom('w', index, byte, at)
        index += 1
      }
      await this.atom((byte & 8) !== 0 ? 'n' : 'e', index, byte, at)
      index += 1
      await this.atom((byte & 4) !== 0 ? 'n' : 'e', index, byte, at)
      return
    }
    if (this.config.cycleAccurate && this.config.minorVersion >= 3 && (byte & 0xfb) === 0x92) {
      await this.atom((byte & 4) !== 0 ? 'n' : 'e', 0, byte, at)
      return
    }
    await this.fail('malformed', at)
  }
  async finishBranch(at: Provenance): Promise<void> {
    await this.out.emit(
      {
        kind: 'branch',
        address: this.branch,
        fragment: this.branchFragment,
        significantBits: this.branchBits,
        isa: this.isa,
        exception: this.exception,
        legacyException: this.legacyException,
        cancel: this.cancel,
        resume: this.resume,
        nonsecure: this.nonsecure,
        hyp: this.hyp,
        altIsa: this.altIsa,
      },
      this.placed(at),
    )
    this.address = this.branch
    this.done()
  }
  async branchByte(byte: u8, at: Provenance): Promise<void> {
    const n = this.count
    let payload: u8 = byte & 0x7f
    let more = (byte & 0x80) !== 0
    let exceptions = false
    let payloadBits: u8 = 7
    if (n === 0) {
      payload = (byte >> 1) & 63
      payloadBits = 6
    }
    if (n === 4) {
      if ((byte & 0x80) !== 0) {
        this.isa = 'arm'
        this.legacyException = (byte >> 3) & 7
        this.cancel = (byte & 0x40) !== 0
        payload = byte & 7
        payloadBits = 3
        more = false
      } else {
        if ((byte & 0x38) === 8) {
          this.isa = 'arm'
          payload = byte & 7
          payloadBits = 3
        } else if ((byte & 0x30) === 0x10) {
          this.isa = 'thumb'
          payload = byte & 15
          payloadBits = 4
        } else if ((byte & 0x20) !== 0) {
          this.isa = 'jazelle'
          payload = byte & 31
          payloadBits = 5
        } else {
          await this.fail('malformed', at)
          return
        }
        exceptions = (byte & 0x40) !== 0
        more = false
      }
    } else if (n > 0 && !more && this.config.branchEncoding === 'alternative') {
      exceptions = (byte & 0x40) !== 0
      payload = byte & 63
      payloadBits = 6
    }
    this.value |= (payload as u64) << (this.shift as u64)
    this.shift += payloadBits
    this.count += 1
    if (more) return
    const isa = this.isa
    let align: u8 = 0
    if (isa === 'arm') align = 2
    if (isa === 'thumb') align = 1
    this.branchBits = n === 4 ? 32 : this.shift + align
    this.branchFragment = (this.value << (align as u64)) as u32
    this.branch = undefined
    if (isa !== undefined) {
      const previous = this.address
      if (n === 4) this.branch = this.branchFragment
      else if (previous !== undefined) {
        const mask: u64 = ((1 as u64) << (this.branchBits as u64)) - 1
        this.branch = (((previous as u64) & (0xffffffff - mask)) |
          (this.branchFragment as u64)) as u32
      }
    }
    if (exceptions) {
      this.mode = 6
      this.excCount = 0
      return
    }
    await this.finishBranch(at)
  }
  async dataDone(at: Provenance): Promise<void> {
    await this.out.emit(
      {
        kind: 'data',
        tag: this.dataTag,
        placeholder: this.placeholder,
        valueTraced: this.valueTraced,
        value: this.placeholder || !this.valueTraced ? undefined : (this.value as u32),
        bytes: this.dataBytes,
        address: this.dataCurrent,
        bigEndian: this.bigEndian,
      },
      this.placed(at),
    )
    this.done()
  }
  async dataValue(at: Provenance): Promise<void> {
    this.value = 0
    this.count = 0
    if (this.dataBytes === 0 || this.placeholder || !this.valueTraced) await this.dataDone(at)
    else this.mode = 10
  }
  async consume(byte: u8, at: Provenance): Promise<void> {
    if (this.mode === 1) {
      if (this.count === 4 && (byte & 0xf0) !== 0) {
        await this.fail('overlong', at)
        return
      }
      this.value |= ((byte & 127) as u64) << (this.shift as u64)
      this.shift += 7
      this.count += 1
      if ((byte & 128) !== 0) return
      await this.out.emit(
        { kind: 'cycle-count', value: this.value as u32, overflow: this.value === 0 },
        this.placed(at),
      )
      if (this.header === 0x70) {
        this.mode = this.config.contextBytes === 0 ? 3 : 2
        this.value = 0
        this.count = 0
        this.shift = 0
      } else this.done()
      return
    }
    if (this.mode === 2 || this.mode === 7) {
      this.value |= (byte as u64) << (this.shift as u64)
      this.shift += 8
      this.count += 1
      if (this.count < this.config.contextBytes) return
      this.context = this.value as u32
      if (this.mode === 7) {
        await this.out.emit({ kind: 'context', value: this.value as u32 }, this.placed(at))
        this.done()
      } else this.mode = 3
      return
    }
    if (this.mode === 3) {
      if ((byte & 1) === 0) {
        await this.fail('malformed', at)
        return
      }
      this.info = byte
      this.lsip = (byte & 128) !== 0
      this.mode = 4
      this.count = 0
      this.shift = 0
      this.value = 0
      return
    }
    if (this.mode === 4) {
      this.value |= (byte as u64) << (this.shift as u64)
      this.shift += 8
      this.count += 1
      if (this.count < 4) return
      const raw: u32 = this.value as u32
      const isa: Isa = (this.info & 16) !== 0 ? 'jazelle' : (raw & 1) !== 0 ? 'thumb' : 'arm'
      const address: u32 = isa === 'jazelle' ? raw : raw & 0xfffffffe
      const nonsecure = (this.info & 8) !== 0
      const hyp = (this.info & 2) !== 0
      const altIsa = (this.info & 4) !== 0
      this.isa = isa
      this.address = address
      this.nonsecure = nonsecure
      this.hyp = hyp
      this.altIsa = altIsa
      await this.out.emit(
        {
          kind: 'isync',
          address,
          context: this.context,
          reason: (this.info >> 5) & 3,
          isa,
          altIsa,
          nonsecure,
          hyp,
          loadStoreInProgress: this.lsip,
        },
        this.placed(at),
      )
      if (this.lsip) {
        this.mode = 12
        this.value = 0
        this.count = 0
        this.shift = 0
        this.exception = 0
        this.legacyException = undefined
        this.cancel = false
        this.resume = 0
      } else this.done()
      return
    }
    if (this.mode === 5 || this.mode === 12) {
      if (this.mode === 12) {
        if ((byte & 1) === 0) {
          await this.fail('malformed', at)
          return
        }
        this.mode = 5
      }
      await this.branchByte(byte, at)
      return
    }
    if (this.mode === 6) {
      if (this.excCount === 0) {
        this.nonsecure = (byte & 1) !== 0
        this.exception = ((byte >> 1) & 15) as u16
        this.cancel = (byte & 32) !== 0
        this.altIsa = (byte & 64) !== 0
        this.excCount = 1
        if ((byte & 128) === 0) await this.finishBranch(at)
        return
      }
      if ((byte & 64) !== 0) {
        if ((byte & 0xb0) !== 0) {
          await this.fail('malformed', at)
          return
        }
        this.resume = byte & 15
        await this.finishBranch(at)
        return
      }
      if (this.excCount !== 1) {
        await this.fail('overlong', at)
        return
      }
      this.exception |= ((byte & 31) as u16) << 4
      this.hyp = (byte & 32) !== 0
      this.excCount = 2
      if ((byte & 128) === 0) await this.finishBranch(at)
      return
    }
    if (this.mode === 8) {
      await this.out.emit({ kind: 'vmid', value: byte }, this.placed(at))
      this.done()
      return
    }
    if (this.mode === 9) {
      if (this.count === 4 && (byte & 0xe0) !== 0) {
        await this.fail('overlong', at)
        return
      }
      const payload: u8 = this.count === 4 ? byte & 15 : byte & 127
      this.value |= (payload as u64) << (this.shift as u64)
      this.shift += this.count === 4 ? 4 : 7
      this.count += 1
      if ((byte & 128) !== 0) return
      if (this.count === 5) this.bigEndian = (byte & 16) !== 0
      const previous = this.dataAddress
      this.dataCurrent = undefined
      if (this.count === 5) this.dataCurrent = this.value as u32
      else if (previous !== undefined) {
        const mask: u64 = ((1 as u64) << (this.shift as u64)) - 1
        this.dataCurrent = (((previous as u64) & (0xffffffff - mask)) | this.value) as u32
      }
      this.dataAddress = this.dataCurrent
      await this.dataValue(at)
      return
    }
    if (this.mode === 10) {
      this.value |= (byte as u64) << ((this.count * 8) as u64)
      this.count += 1
      if (this.count === this.dataBytes) await this.dataDone(at)
      return
    }
    if (this.mode === 11) {
      const last64 = this.config.timestampBits === 64 && this.count === 8
      const last48 = this.config.timestampBits === 48 && this.count === 6
      if (last48 && (byte & 0xc0) !== 0) {
        await this.fail('overlong', at)
        return
      }
      this.value |= ((last64 ? byte : byte & 127) as u64) << (this.shift as u64)
      this.shift += last64 ? 8 : last48 ? 6 : 7
      this.count += 1
      if (!last64 && (byte & 128) !== 0) return
      await this.out.emit(
        {
          kind: 'timestamp',
          fragment: this.value,
          significantBits: this.shift,
          clockChange: (this.header & 4) !== 0,
        },
        this.placed(at),
      )
      this.done()
      return
    }
    if (byte === 0) {
      if (this.zeros === 0) this.zeroAt = at
      if (this.zeros < 255) this.zeros += 1
      return
    }
    if (byte === 0x80 && this.zeros >= 5) {
      const zero = this.zeroAt
      this.reset()
      this.aligned = true
      if (zero !== undefined)
        await this.out.emit({ kind: 'sync' }, over(zero.span.start, at.span.end, at.settled))
      return
    }
    if (this.zeros !== 0) {
      const zero = this.zeroAt
      this.zeros = 0
      this.zeroAt = undefined
      if (this.aligned && zero !== undefined) {
        await this.fail('bad-sync', over(zero.span.start, at.span.end, at.settled))
        return
      }
    }
    if (!this.aligned) return
    if ((byte & 1) !== 0) {
      this.begin(5, byte, at)
      this.exception = 0
      this.legacyException = undefined
      this.cancel = false
      this.resume = 0
      await this.branchByte(byte, at)
      return
    }
    if ((byte & 128) !== 0) {
      await this.atoms(byte, at)
      return
    }
    if (byte === 4 || byte === 0x70) {
      this.begin(1, byte, at)
      return
    }
    if (byte === 8) {
      this.begin(this.config.contextBytes === 0 ? 3 : 2, byte, at)
      return
    }
    if (byte === 0x6e) {
      if (this.config.contextBytes === 0) {
        await this.fail('malformed', at)
        return
      }
      this.begin(7, byte, at)
      return
    }
    if (byte === 0x3c && this.config.minorVersion >= 5) {
      this.begin(8, byte, at)
      return
    }
    if ((byte === 0x42 || byte === 0x46) && this.config.minorVersion >= 5) {
      this.begin(11, byte, at)
      return
    }
    let marker: Marker | undefined = undefined
    if (byte === 0x0c) marker = 'trigger'
    if (byte === 0x50) marker = 'store-failed'
    if (byte === 0x62) marker = 'data-suppressed'
    if (byte === 0x66) marker = 'ignore'
    if (byte === 0x76) marker = 'exception-exit'
    if (byte === 0x7e) marker = 'exception-entry'
    if (marker !== undefined) {
      await this.out.emit({ kind: 'marker', marker }, at)
      return
    }
    const normal = (byte & 0xd3) === 2
    const placeholder = (byte & 0xd3) === 0x50 && ((byte >> 2) & 3) !== 0
    const returned = (byte & 0x93) === 0 && ((byte >> 5) & 3) !== 0
    const notTraced = byte === 0x6a || byte === 0x7a
    if (normal || placeholder || returned || notTraced) {
      this.begin(10, byte, at)
      this.placeholder = placeholder
      this.valueTraced = !notTraced
      this.dataTag = placeholder ? (byte >> 2) & 3 : returned ? (byte >> 5) & 3 : 0
      const size: u8 = (byte >> 2) & 3
      this.dataBytes = placeholder || notTraced ? 0 : size === 3 ? 4 : size
      this.dataCurrent = undefined
      if (!returned && (byte & (notTraced ? 16 : 32)) !== 0 && this.config.dataAddresses) {
        this.mode = 9
        return
      }
      await this.dataValue(at)
      return
    }
    this.begin(13, byte, at)
    await this.fail('unsupported-header', at)
  }
  override async run(): Promise<void> {
    for (;;) {
      const item = await this.input.next()
      const event = item.event
      if (event.kind === 'error') {
        await this.fail('input-error', item.at)
        continue
      }
      if (event.kind === 'byte') await this.consume(event.value, item.at)
    }
  }
}
export const armEtmv3 = defineStreamDecoder({
  name: 'arm-etmv3',
  category: 'device',
  config: {
    defaults: {
      startAligned: false,
      contextBytes: 0,
      cycleAccurate: false,
      minorVersion: 5,
      branchEncoding: 'alternative',
      dataAddresses: false,
      timestampBits: 48,
    },
    validate,
  },
  input: traceByteEvents,
  output: armEtmv3Events,
  decoder: ArmEtmv3,
  locales: {
    'en-US': {
      name: 'Arm ETMv3',
      summary: 'ETMv3 packet fields, atoms, observed branch addresses and data',
      fields: {
        startAligned: 'Assume packet alignment',
        contextBytes: 'Context ID bytes',
        cycleAccurate: 'Cycle-accurate atom encoding',
        minorVersion: 'ETMv3 minor version',
        branchEncoding: 'Branch address encoding',
        dataAddresses: 'Data address tracing enabled',
        timestampBits: 'Timestamp width',
      },
      values: {
        branchEncoding: { original: 'Original encoding', alternative: 'Alternative encoding' },
      },
      events: {
        sync: 'Alignment synchronization',
        atom: 'Instruction atom',
        marker: 'Trace marker',
        'cycle-count': 'Cycle count',
        isync: 'Instruction synchronization',
        branch: 'Branch',
        context: 'Context ID',
        vmid: 'VMID',
        data: 'Data',
        timestamp: 'Timestamp fragment',
        error: 'Error',
      },
      eventFields: {
        atom: 'Atom',
        index: 'Atom index',
        header: 'Header',
        marker: 'Marker',
        value: 'Value',
        overflow: 'Counter overflow',
        address: 'Observed address',
        context: 'Context',
        reason: 'Reason',
        isa: 'Instruction set',
        altIsa: 'Alternative ISA',
        nonsecure: 'Non-secure',
        hyp: 'Hypervisor',
        loadStoreInProgress: 'Load/store in progress',
        fragment: 'Transmitted fragment',
        significantBits: 'Significant bits',
        exception: 'Exception',
        legacyException: 'Legacy exception encoding',
        cancel: 'Previous instruction canceled',
        resume: 'Resume',
        tag: 'Data tag',
        placeholder: 'Outstanding data placeholder',
        valueTraced: 'Value traced',
        bytes: 'Value bytes',
        bigEndian: 'BE-8 transfer',
        clockChange: 'Clock changed',
      },
    },
    'zh-CN': {
      name: 'Arm ETMv3',
      summary: 'ETMv3 数据包字段、执行原子、观察到的分支地址和数据',
      fields: {
        startAligned: '假定数据包对齐',
        contextBytes: '上下文标识字节数',
        cycleAccurate: '周期精确原子编码',
        minorVersion: 'ETMv3 次版本',
        branchEncoding: '分支地址编码',
        dataAddresses: '已启用数据地址跟踪',
        timestampBits: '时间戳宽度',
      },
      values: { branchEncoding: { original: '原始编码', alternative: '替代编码' } },
      events: {
        sync: '对齐同步',
        atom: '指令原子',
        marker: '跟踪标记',
        'cycle-count': '周期数',
        isync: '指令同步',
        branch: '分支',
        context: '上下文标识',
        vmid: 'VMID',
        data: '数据',
        timestamp: '时间戳片段',
        error: '错误',
      },
      eventFields: {
        atom: '原子',
        index: '原子序号',
        header: '头部',
        marker: '标记',
        value: '值',
        overflow: '计数器溢出',
        address: '观察到的地址',
        context: '上下文',
        reason: '原因',
        isa: '指令集',
        altIsa: '替代指令集',
        nonsecure: '非安全状态',
        hyp: '管理程序',
        loadStoreInProgress: '加载存储进行中',
        fragment: '传输片段',
        significantBits: '有效位数',
        exception: '异常',
        legacyException: '旧式异常编码',
        cancel: '前一指令已取消',
        resume: '恢复',
        tag: '数据标签',
        placeholder: '待返回数据占位',
        valueTraced: '已跟踪值',
        bytes: '值字节数',
        bigEndian: 'BE-8 传输',
        clockChange: '时钟已变化',
      },
    },
  },
})
