// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Integer assertions select IR widths and truncation; TypeScript aliases alone do not encode these semantics. */
/** STM32F1 two-TAP profile from ST RM0008 Rev21 §§31.5–31.7;
 * ADIv5 scan fields from Arm IHI0031A §4.4. No target effects inferred. */
import type { bool, u8, u16, u32, u64, Provenance, Tick, Range } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
import { jtagEvents, type JtagEvent } from './jtag-chain'
// eslint-disable-next-line @typescript-eslint/no-empty-object-type -- The DSL declares an exact empty configuration record.
export interface JtagStm32Config {}
export type ErrorReason =
  | 'aborted'
  | 'invalid-length'
  | 'missing-line'
  | 'unknown-instruction'
  | 'offset'
  | 'invalid-capture'
export type Density = 'low' | 'medium' | 'high' | 'xl' | 'connectivity' | 'unknown'
export type JtagStm32Event =
  | { kind: 'reset' }
  | { kind: 'instruction'; scan: u64; mcu: u8; bsc: u8; captureOk: bool }
  | {
      kind: 'idcode'
      scan: u64
      boundaryScan: bool
      value: u32
      version: u8
      part: u16
      manufacturer: u16
      lsbOk: bool
      density: Density
    }
  | {
      kind: 'access'
      scan: u64
      ap: bool
      read: bool
      addr: u8
      dataIn: u32
      ackBits: u8
      responseData: u32
      previousScan: u64 | undefined
      accepted: bool | undefined
    }
  | { kind: 'abort'; scan: u64; value: u32; lowBits: u8 }
  | { kind: 'bypass'; scan: u64; mcu: bool | undefined; bsc: bool | undefined }
  | { kind: 'error'; scan: u64; bits: u32; reason: ErrorReason }
export const jtagStm32Events = eventSchema<JtagStm32Event>('jtag-stm32.events', 1).roles({
  reset: 'framing',
  instruction: 'command',
  idcode: 'address',
  access: 'data',
  abort: 'command',
  bypass: 'data',
  error: 'error',
})
function density(value: u32): Density {
  const part: u16 = ((value >> 12) & 0xffff) as u16
  if ((value & 0xfff) !== 0x41) return 'unknown'
  if (part === 0x6412) return 'low'
  if (part === 0x6410) return 'medium'
  if (part === 0x6414) return 'high'
  if (part === 0x6430) return 'xl'
  if (part === 0x6418) return 'connectivity'
  return 'unknown'
}
class JtagStm32 extends StreamDecoder<JtagStm32Config, JtagEvent, JtagStm32Event> {
  mcu: u8 | undefined = undefined
  bsc: u8 | undefined = undefined
  pending: u64 | undefined = undefined
  scan: u64 | undefined = undefined
  ir = false
  bits: u32 = 0
  inputBits: u64 = 0
  outputBits: u64 = 0
  first: Provenance | undefined = undefined
  malformed: ErrorReason | undefined = undefined
  override hold(): Tick | undefined {
    const first = this.first
    if (first !== undefined) return first.span.start
    return undefined
  }
  clear(): void {
    this.mcu = undefined
    this.bsc = undefined
    this.pending = undefined
    this.scan = undefined
    this.first = undefined
  }
  async abort(tick: Tick): Promise<void> {
    const scan = this.scan
    const first = this.first
    if (scan !== undefined)
      await this.out.emit(
        { kind: 'error', scan, bits: this.bits, reason: 'aborted' },
        first === undefined ? over(tick, tick, tick) : over(first.span.start, tick, tick),
      )
    this.clear()
  }
  override async onGap(range: Range): Promise<void> {
    await this.abort(range.start)
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.abort(tick)
  }
  bit(position: u32, input: u8, output: u8): void {
    if (position < 64) {
      this.inputBits |= (input as u64) << (position as u64)
      this.outputBits |= (output as u64) << (position as u64)
    }
  }
  async identify(scan: u64, value: u32, boundaryScan: bool, at: Provenance): Promise<void> {
    await this.out.emit(
      {
        kind: 'idcode',
        scan,
        boundaryScan,
        value,
        version: (value >> 28) as u8,
        part: ((value >> 12) & 0xffff) as u16,
        manufacturer: ((value >> 1) & 0x7ff) as u16,
        lsbOk: (value & 1) !== 0,
        density: boundaryScan ? density(value) : 'unknown',
      },
      at,
    )
  }
  async fail(scan: u64, reason: ErrorReason, at: Provenance): Promise<void> {
    await this.out.emit({ kind: 'error', scan, bits: this.bits, reason }, at)
    this.clear()
  }
  async finish(scan: u64, at: Provenance): Promise<void> {
    if (this.ir) {
      if (this.bits !== 9) {
        await this.fail(scan, 'invalid-length', at)
        return
      }
      const captureOk = (this.outputBits & 3) === 1 && ((this.outputBits >> 4) & 3) === 1
      const mcu: u8 = (this.inputBits & 15) as u8
      const bsc: u8 = ((this.inputBits >> 4) & 31) as u8
      await this.out.emit({ kind: 'instruction', scan, mcu, bsc, captureOk }, at)
      if (!captureOk) {
        await this.fail(scan, 'invalid-capture', at)
        return
      }
      this.mcu = mcu
      this.bsc = bsc
      return
    }
    const mcu = this.mcu
    const bsc = this.bsc
    if (mcu === undefined) {
      await this.fail(scan, 'unknown-instruction', at)
      return
    }
    if (bsc === undefined) {
      await this.fail(scan, 'unknown-instruction', at)
      return
    }
    let mcuBits: u32 = 1
    if (mcu === 14) mcuBits = 32
    else if (mcu === 8 || mcu === 10 || mcu === 11) mcuBits = 35
    else if (mcu !== 15) {
      await this.fail(scan, 'unknown-instruction', at)
      return
    }
    let bscBits: u32 = 1
    if (bsc === 1) bscBits = 32
    else if (bsc !== 31) {
      await this.fail(scan, 'unknown-instruction', at)
      return
    }
    if (mcuBits + bscBits > 64) {
      await this.fail(scan, 'unknown-instruction', at)
      return
    }
    if (this.bits !== mcuBits + bscBits) {
      await this.fail(scan, 'invalid-length', at)
      return
    }
    if (mcu === 14) await this.identify(scan, this.outputBits as u32, false, at)
    if (bsc === 1) await this.identify(scan, (this.outputBits >> (mcuBits as u64)) as u32, true, at)
    if (mcu === 8) {
      await this.out.emit(
        {
          kind: 'abort',
          scan,
          value: (this.inputBits >> 3) as u32,
          lowBits: (this.inputBits & 7) as u8,
        },
        at,
      )
      this.pending = undefined
    }
    if (mcu === 10 || mcu === 11) {
      const ackBits: u8 = (this.outputBits & 7) as u8
      const previousScan = this.pending
      let accepted: bool | undefined = undefined
      if (ackBits === 1) accepted = false
      if (ackBits === 2) accepted = true
      await this.out.emit(
        {
          kind: 'access',
          scan,
          ap: mcu === 11,
          read: (this.inputBits & 1) !== 0,
          addr: ((this.inputBits & 6) << 1) as u8,
          dataIn: (this.inputBits >> 3) as u32,
          ackBits,
          responseData: (this.outputBits >> 3) as u32,
          previousScan,
          accepted,
        },
        at,
      )
      if (ackBits === 2) this.pending = scan
      else if (ackBits !== 1) this.pending = undefined
    }
    if (mcu === 15 || bsc === 31)
      await this.out.emit(
        {
          kind: 'bypass',
          scan,
          mcu: mcu === 15 ? (this.outputBits & 1) !== 0 : undefined,
          bsc: bsc === 31 ? ((this.outputBits >> (mcuBits as u64)) & 1) !== 0 : undefined,
        },
        at,
      )
  }

  override async run(): Promise<void> {
    for (;;) {
      const item = await this.input.next()
      const event = item.event
      if (event.kind === 'reset') {
        this.clear()
        this.mcu = 14
        this.bsc = 1
        await this.out.emit({ kind: 'reset' }, item.at)
      }
      if (event.kind === 'scan-begin') {
        this.scan = event.scan
        this.ir = event.register === 'ir'
        this.bits = 0
        this.inputBits = 0
        this.outputBits = 0
        this.first = item.at
        this.malformed = undefined
      }
      if (event.kind === 'shift') {
        if (this.scan !== event.scan) continue
        if (this.bits !== event.offset) this.malformed = 'offset'
        const input = event.tdi
        const output = event.tdo
        if (input !== undefined) {
          if (output !== undefined) {
            let index: u16 = 0
            for (const a of input) {
              const b = output[index >> 3]
              for (let shift: u8 = 0; shift < 8; shift += 1) {
                if (index < event.bits) {
                  if (b !== undefined)
                    this.bit(
                      event.offset + index,
                      ((a >> (shift as u64)) & 1) as u8,
                      ((b >> (shift as u64)) & 1) as u8,
                    )
                  else this.malformed = 'missing-line'
                }
                index += 1
              }
            }
          } else this.malformed = 'missing-line'
        } else this.malformed = 'missing-line'

        this.bits = event.offset + event.bits
      }
      if (event.kind === 'scan-end') {
        if (this.scan !== event.scan) continue
        const first = this.first
        const at =
          first === undefined ? item.at : over(first.span.start, item.at.span.end, item.at.settled)
        if (event.reason !== 'complete') this.malformed = 'aborted'
        if (event.bits !== this.bits) this.malformed = 'offset'
        const malformed = this.malformed
        if (malformed !== undefined) {
          await this.out.emit(
            { kind: 'error', scan: event.scan, bits: this.bits, reason: malformed },
            at,
          )
          this.clear()
        } else {
          await this.finish(event.scan, at)
          this.scan = undefined
          this.first = undefined
        }
      }
    }
  }
}
export const jtagStm32 = defineStreamDecoder({
  name: 'jtag-stm32',
  category: 'device',
  config: { defaults: {} },
  input: jtagEvents,
  requires: () => ({ tdi: true, tdo: true }),
  output: jtagStm32Events,
  decoder: JtagStm32,
  locales: {
    'en-US': {
      name: 'STM32F1 JTAG',
      summary: 'Observed MCU/BSC chain IDs and ADIv5 access fields',
      fields: {},
      events: {
        reset: 'Reset',
        instruction: 'Instruction',
        idcode: 'Device ID',
        access: 'DP/AP access',
        abort: 'Abort request',
        bypass: 'Bypass',
        error: 'Error',
      },
      eventFields: {
        scan: 'Scan',
        mcu: 'MCU TAP',
        bsc: 'Boundary scan TAP',
        captureOk: 'IR capture valid',
        boundaryScan: 'Boundary scan ID',
        value: 'Value',
        version: 'Version',
        part: 'Part',
        manufacturer: 'Manufacturer',
        lsbOk: 'ID bit zero valid',
        density: 'Device density',
        ap: 'AP access',
        read: 'Read',
        addr: 'Register address',
        dataIn: 'Request data',
        ackBits: 'ACK bits',
        responseData: 'Previous response data',
        previousScan: 'Previous accepted scan',
        accepted: 'Request accepted',
        lowBits: 'Low bits',
        bits: 'Bits',
        reason: 'Reason',
      },
    },
    'zh-CN': {
      name: 'STM32F1 JTAG',
      summary: '观察到的 MCU/BSC 链标识与 ADIv5 访问字段',
      fields: {},
      events: {
        reset: '复位',
        instruction: '指令',
        idcode: '器件标识',
        access: 'DP/AP 访问',
        abort: '中止请求',
        bypass: '旁路',
        error: '错误',
      },
      eventFields: {
        scan: '扫描',
        mcu: 'MCU TAP',
        bsc: '边界扫描 TAP',
        captureOk: 'IR 捕获有效',
        boundaryScan: '边界扫描标识',
        value: '值',
        version: '版本',
        part: '型号',
        manufacturer: '制造商',
        lsbOk: '标识最低位有效',
        density: '器件容量',
        ap: 'AP 访问',
        read: '读取',
        addr: '寄存器地址',
        dataIn: '请求数据',
        ackBits: 'ACK 位',
        responseData: '前次响应数据',
        previousScan: '前次已接受扫描',
        accepted: '请求已接受',
        lowBits: '低位',
        bits: '位数',
        reason: '原因',
      },
    },
  },
})
