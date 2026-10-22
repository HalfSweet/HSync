// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Integer assertions select IR widths and truncation; TypeScript aliases alone do not encode these semantics. */

/** MIPS32 EJTAG registers, independently from MIPS MD00249 02.03 §§8.4–8.5
 * https://hades.mech.northwestern.edu/images/c/c1/M4K_Software_Manual.pdf
 * and Microchip DS60001145 §20.2. No target memory effects are inferred.
 */
import type { bool, u8, u16, u32, u64, Provenance, Tick, Range } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
import { jtagEvents, type JtagEvent } from './jtag-chain'
export interface JtagEjtagConfig {
  irBits: u8
}
export type RegisterName =
  | 'idcode'
  | 'impcode'
  | 'address'
  | 'data'
  | 'control'
  | 'all'
  | 'ejtagboot'
  | 'normalboot'
  | 'fastdata'
  | 'bypass'
  | 'unknown'
export type ErrorReason =
  | 'aborted'
  | 'invalid-length'
  | 'missing-line'
  | 'unknown-instruction'
  | 'offset'
  | 'invalid-capture'
export type JtagEjtagEvent =
  | { kind: 'reset' }
  | { kind: 'instruction'; scan: u64; value: u8; register: RegisterName; captureOk: bool }
  | { kind: 'register'; scan: u64; register: RegisterName; dataIn: u32; dataOut: u32 }
  | {
      kind: 'idcode'
      scan: u64
      value: u32
      version: u8
      part: u16
      manufacturer: u16
      lsbOk: bool
    }
  | { kind: 'implementation'; scan: u64; value: u32; version: u8 }
  | {
      kind: 'control'
      scan: u64
      dataIn: u32
      dataOut: u32
      pracc: bool
      prnw: bool
      resetOccurred: bool
      debugMode: bool
      doze: bool
      halt: bool
    }
  | {
      kind: 'processor-completion-request'
      scan: u64
      address: u32 | undefined
      dataIn: u32 | undefined
      dataOut: u32 | undefined
      prnw: bool
    }
  | {
      kind: 'fastdata'
      scan: u64
      completionRequested: bool
      successFlag: bool
      completed: bool
      dataIn: u32
      dataOut: u32
    }
  | { kind: 'bypass'; scan: u64; dataIn: bool; dataOut: bool }
  | { kind: 'error'; scan: u64; bits: u32; reason: ErrorReason }
export const jtagEjtagEvents = eventSchema<JtagEjtagEvent>('jtag-ejtag.events', 1).roles({
  reset: 'framing',
  instruction: 'control',
  register: 'data',
  idcode: 'address',
  implementation: 'control',
  control: 'control',
  'processor-completion-request': 'control',
  fastdata: 'data',
  bypass: 'data',
  error: 'error',
})
function name(value: u8, bypass: u8): RegisterName {
  if (value === bypass) return 'bypass'
  switch (value) {
    case 1:
      return 'idcode'
    case 3:
      return 'impcode'
    case 8:
      return 'address'
    case 9:
      return 'data'
    case 10:
      return 'control'
    case 11:
      return 'all'
    case 12:
      return 'ejtagboot'
    case 13:
      return 'normalboot'
    case 14:
      return 'fastdata'
    default:
      return 'unknown'
  }
}
function validate(config: JtagEjtagConfig): string | undefined {
  if (config.irBits < 5 || config.irBits > 8) return 'decode.config.ejtagIrBits'
  return undefined
}
class JtagEjtag extends StreamDecoder<JtagEjtagConfig, JtagEvent, JtagEjtagEvent> {
  instruction: u8 | undefined = undefined
  scan: u64 | undefined = undefined
  ir = false
  bits: u32 = 0
  first: Provenance | undefined = undefined
  malformed: ErrorReason | undefined = undefined
  in0: u32 = 0
  in1: u32 = 0
  in2: u32 = 0
  out0: u32 = 0
  out1: u32 = 0
  out2: u32 = 0
  pending = false
  address: u32 | undefined = undefined
  dataIn: u32 | undefined = undefined
  dataOut: u32 | undefined = undefined
  override hold(): Tick | undefined {
    const first = this.first
    if (first !== undefined) return first.span.start
    return undefined
  }
  clear(): void {
    this.instruction = undefined
    this.scan = undefined
    this.first = undefined
    this.pending = false
    this.address = undefined
    this.dataIn = undefined
    this.dataOut = undefined
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
    const shift: u64 = (position & 31) as u64
    if (position < 32) {
      this.in0 |= (input as u32) << shift
      this.out0 |= (output as u32) << shift
    } else if (position < 64) {
      this.in1 |= (input as u32) << shift
      this.out1 |= (output as u32) << shift
    } else if (position < 96) {
      this.in2 |= (input as u32) << shift
      this.out2 |= (output as u32) << shift
    }
  }
  async register(
    scan: u64,
    register: RegisterName,
    input: u32,
    output: u32,
    at: Provenance,
  ): Promise<void> {
    await this.out.emit({ kind: 'register', scan, register, dataIn: input, dataOut: output }, at)
  }
  async control(scan: u64, all: bool, at: Provenance): Promise<void> {
    const input = this.in0
    const output = this.out0
    const pracc = (output & 0x40000) !== 0
    const prnw = (output & 0x80000) !== 0
    if (pracc && !this.pending) {
      this.address = undefined
      this.dataIn = undefined
      this.dataOut = undefined
    }
    this.pending = pracc
    if (all && pracc) {
      this.address = this.out2
      this.dataIn = this.in1
      this.dataOut = this.out1
    }
    await this.out.emit(
      {
        kind: 'control',
        scan,
        dataIn: input,
        dataOut: output,
        pracc,
        prnw,
        resetOccurred: (output & 0x80000000) !== 0,
        debugMode: (output & 8) !== 0,
        doze: (output & 0x400000) !== 0,
        halt: (output & 0x200000) !== 0,
      },
      at,
    )
    if (pracc && (input & 0x40000) === 0)
      await this.out.emit(
        {
          kind: 'processor-completion-request',
          scan,
          address: this.address,
          dataIn: this.dataIn,
          dataOut: this.dataOut,
          prnw,
        },
        at,
      )
    if (!pracc) {
      this.address = undefined
      this.dataIn = undefined
      this.dataOut = undefined
    }
  }
  async finish(scan: u64, at: Provenance): Promise<void> {
    const bypass: u8 = (((1 as u16) << this.config.irBits) - 1) as u8
    if (this.ir) {
      if (this.bits !== this.config.irBits) {
        await this.out.emit({ kind: 'error', scan, bits: this.bits, reason: 'invalid-length' }, at)
        this.instruction = undefined
        return
      }
      const value: u8 = this.in0 as u8
      const captureOk = (this.out0 & 3) === 1
      await this.out.emit(
        { kind: 'instruction', scan, value, register: name(value, bypass), captureOk },
        at,
      )
      this.instruction = captureOk ? value : undefined
      if (!captureOk)
        await this.out.emit({ kind: 'error', scan, bits: this.bits, reason: 'invalid-capture' }, at)
      return
    }
    const instruction = this.instruction
    if (instruction === undefined) {
      await this.out.emit(
        { kind: 'error', scan, bits: this.bits, reason: 'unknown-instruction' },
        at,
      )
      return
    }
    const register = name(instruction, bypass)
    if (register === 'unknown') {
      await this.out.emit(
        { kind: 'error', scan, bits: this.bits, reason: 'unknown-instruction' },
        at,
      )
      return
    }
    const expected: u32 =
      register === 'all'
        ? 96
        : register === 'fastdata'
          ? 33
          : register === 'bypass' || register === 'ejtagboot' || register === 'normalboot'
            ? 1
            : 32
    if (this.bits !== expected) {
      await this.out.emit({ kind: 'error', scan, bits: this.bits, reason: 'invalid-length' }, at)
      this.pending = false
      this.address = undefined
      this.dataIn = undefined
      this.dataOut = undefined
      return
    }
    if (register === 'fastdata') {
      const completionRequested = (this.in0 & 1) === 0
      const successFlag = (this.out0 & 1) !== 0
      await this.out.emit(
        {
          kind: 'fastdata',
          scan,
          completionRequested,
          successFlag,
          completed: completionRequested && successFlag,
          dataIn: (this.in0 >> 1) | ((this.in1 & 1) << 31),
          dataOut: (this.out0 >> 1) | ((this.out1 & 1) << 31),
        },
        at,
      )
      if (completionRequested && successFlag) {
        this.pending = false
        this.address = undefined
        this.dataIn = undefined
        this.dataOut = undefined
      }
      return
    }
    if (register === 'bypass' || register === 'ejtagboot' || register === 'normalboot') {
      await this.out.emit(
        { kind: 'bypass', scan, dataIn: (this.in0 & 1) !== 0, dataOut: (this.out0 & 1) !== 0 },
        at,
      )
      return
    }
    if (register === 'all') {
      await this.register(scan, 'control', this.in0, this.out0, at)
      await this.register(scan, 'data', this.in1, this.out1, at)
      await this.register(scan, 'address', this.in2, this.out2, at)
      await this.control(scan, true, at)
      return
    }
    await this.register(scan, register, this.in0, this.out0, at)
    if (register === 'idcode')
      await this.out.emit(
        {
          kind: 'idcode',
          scan,
          value: this.out0,
          version: (this.out0 >> 28) as u8,
          part: ((this.out0 >> 12) & 0xffff) as u16,
          manufacturer: ((this.out0 >> 1) & 0x7ff) as u16,
          lsbOk: (this.out0 & 1) !== 0,
        },
        at,
      )
    if (register === 'impcode')
      await this.out.emit(
        { kind: 'implementation', scan, value: this.out0, version: (this.out0 >> 29) as u8 },
        at,
      )
    if (register === 'control') await this.control(scan, false, at)
    if (register === 'address') this.address = this.pending ? this.out0 : undefined
    if (register === 'data') {
      this.dataIn = this.pending ? this.in0 : undefined
      this.dataOut = this.pending ? this.out0 : undefined
    }
  }
  override async run(): Promise<void> {
    for (;;) {
      const item = await this.input.next()
      const event = item.event
      if (event.kind === 'reset') {
        this.clear()
        this.instruction = 1
        await this.out.emit({ kind: 'reset' }, item.at)
      }
      if (event.kind === 'scan-begin') {
        this.scan = event.scan
        this.ir = event.register === 'ir'
        this.bits = 0
        this.in0 = 0
        this.in1 = 0
        this.in2 = 0
        this.out0 = 0
        this.out1 = 0
        this.out2 = 0
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
export const jtagEjtag = defineStreamDecoder({
  name: 'jtag-ejtag',
  category: 'device',
  config: { defaults: { irBits: 5 }, validate },
  input: jtagEvents,
  requires: () => ({ tdi: true, tdo: true }),
  output: jtagEjtagEvents,
  locales: {
    'en-US': {
      name: 'MIPS32 EJTAG',
      summary: 'Observed EJTAG registers, processor-access requests and FASTDATA flags',
      fields: { irBits: 'Instruction register width' },
      events: {
        reset: 'Reset',
        instruction: 'Instruction',
        register: 'Register',
        idcode: 'Device ID',
        implementation: 'Implementation',
        control: 'Control status',
        'processor-completion-request': 'Processor access completion request',
        fastdata: 'FASTDATA',
        bypass: 'Bypass',
        error: 'Error',
      },
      eventFields: {
        scan: 'Scan',
        value: 'Value',
        register: 'Register',
        captureOk: 'IR capture valid',
        dataIn: 'Probe data in',
        dataOut: 'Captured data out',
        version: 'Version',
        part: 'Part',
        manufacturer: 'Manufacturer',
        lsbOk: 'ID low bit valid',
        pracc: 'Processor access pending',
        prnw: 'PrNW flag',
        resetOccurred: 'Reset occurred',
        debugMode: 'Debug mode',
        doze: 'Doze',
        halt: 'Halt',
        address: 'Observed address',
        completionRequested: 'Completion requested',
        successFlag: 'Success flag',
        completed: 'Transport completed',
        bits: 'Bits',
        reason: 'Reason',
      },
    },
    'zh-CN': {
      name: 'MIPS32 EJTAG',
      summary: 'EJTAG 寄存器、处理器访问请求及 FASTDATA 标志',
      fields: { irBits: '指令寄存器宽度' },
      events: {
        reset: '复位',
        instruction: '指令',
        register: '寄存器',
        idcode: '设备 ID',
        implementation: '实现',
        control: '控制状态',
        'processor-completion-request': '处理器访问完成请求',
        fastdata: 'FASTDATA',
        bypass: '旁路',
        error: '错误',
      },
      eventFields: {
        scan: '扫描',
        value: '值',
        register: '寄存器',
        captureOk: 'IR 捕获有效',
        dataIn: '探针写入数据',
        dataOut: '捕获读出数据',
        version: '版本',
        part: '部件',
        manufacturer: '制造商',
        lsbOk: 'ID 最低位有效',
        pracc: '处理器访问待处理',
        prnw: 'PrNW 标志',
        resetOccurred: '已复位',
        debugMode: '调试模式',
        doze: '休眠',
        halt: '暂停',
        address: '观察到的地址',
        completionRequested: '请求完成',
        successFlag: '成功标志',
        completed: '传输完成',
        bits: '位数',
        reason: '原因',
      },
    },
  },
  decoder: JtagEjtag,
})
