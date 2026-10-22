// SPDX-License-Identifier: Apache-2.0
/** OScan1 serial transport from Arm's T4 waveform and SEGGER's short activation
 * sequence. Independently authored; GPL reference consulted for capability audit only. */
import type { bool, Channel, Tick, Range, u8, u16, u32, u64, Bounded } from 'uiharu'
import { LogicDecoder, defineLogicDecoder, mask, max, atLeast, one, over, fromBool } from 'uiharu'
import { jtagEvents, type JtagEvent, type TapState, type Register } from './jtag-chain'
export interface CjtagConfig {
  clock: Channel
  data: Channel
  startAligned: bool
  format: Format
  readyOnes: u8
  delayCycles: u8
  maxStallCycles: u16
}
export type Format = 'oscan0' | 'oscan1'
function validate(c: CjtagConfig): string | undefined {
  if (c.clock >= 64 || c.data >= 64 || c.clock === c.data) return 'decode.config.channels'
  if (c.readyOnes < 1 || c.readyOnes > 4 || c.delayCycles > 2 || c.maxStallCycles < 4)
    return 'decode.config.cjtagTiming'
  return undefined
}
function next(state: TapState, tms: bool): TapState {
  if (state === 'test-logic-reset') return tms ? 'test-logic-reset' : 'run-test-idle'
  if (state === 'run-test-idle') return tms ? 'select-dr-scan' : 'run-test-idle'
  if (state === 'select-dr-scan') return tms ? 'select-ir-scan' : 'capture-dr'
  if (state === 'capture-dr') return tms ? 'exit1-dr' : 'shift-dr'
  if (state === 'shift-dr') return tms ? 'exit1-dr' : 'shift-dr'
  if (state === 'exit1-dr') return tms ? 'update-dr' : 'pause-dr'
  if (state === 'pause-dr') return tms ? 'exit2-dr' : 'pause-dr'
  if (state === 'exit2-dr') return tms ? 'update-dr' : 'shift-dr'
  if (state === 'update-dr') return tms ? 'select-dr-scan' : 'run-test-idle'
  if (state === 'select-ir-scan') return tms ? 'test-logic-reset' : 'capture-ir'
  if (state === 'capture-ir') return tms ? 'exit1-ir' : 'shift-ir'
  if (state === 'shift-ir') return tms ? 'exit1-ir' : 'shift-ir'
  if (state === 'exit1-ir') return tms ? 'update-ir' : 'pause-ir'
  if (state === 'pause-ir') return tms ? 'exit2-ir' : 'pause-ir'
  if (state === 'exit2-ir') return tms ? 'update-ir' : 'shift-ir'
  return tms ? 'select-dr-scan' : 'run-test-idle'
}
class Cjtag extends LogicDecoder<CjtagConfig, JtagEvent> {
  readonly watch = mask(this.config.clock, this.config.data)
  active = this.config.startAligned
  activeFormat: Format = this.config.format
  ready: u8 = 0
  stalls: u16 = 0
  delays: u8 = 0
  tdoBit: u8 = 0
  selecting = false
  selection: u64 = 0
  selectionBits: u8 = 0
  escapes: u8 = 0
  selectionLength: u8 = 12
  zeroScans: u8 = 0
  commandLevel = false
  commandStep: u8 = 0
  commandOpcode: u32 = 0
  commandParameter: u32 = 0
  commandBits: u32 = 0
  checkBits: u8 = 0
  checkValue: u8 = 0
  checking = false
  phase: u8 = 0
  first: Tick | undefined = undefined
  tdiBit: u8 = 0
  tms = false
  state: TapState = 'test-logic-reset'
  ones: u8 = 0
  scan: u64 | undefined = undefined
  nextScan: u64 = 1
  bits: u32 = 0
  override hold(): Tick | undefined {
    return this.first
  }
  async cut(tick: Tick): Promise<void> {
    const scan = this.scan
    if (scan !== undefined)
      await this.out.emit({ kind: 'scan-end', scan, bits: this.bits, reason: 'aborted' }, one(tick))
    this.scan = undefined
    this.first = undefined
    this.phase = 0
    this.ones = 0
    this.state = 'test-logic-reset'
  }
  override async onGap(range: Range): Promise<void> {
    await this.cut(range.start)
    this.active = false
    this.selecting = false
    this.escapes = 0
    this.checking = false
    this.commandLevel = false
    this.zeroScans = 0
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
  }
  command(state: TapState, previous: TapState): void {
    if (state === 'test-logic-reset') {
      this.zeroScans = 0
      this.commandLevel = false
      this.commandStep = 0
    }
    if (state === 'capture-dr' || (state === 'shift-dr' && previous === 'exit2-dr'))
      this.commandBits = 0
    if (previous === 'shift-dr' && this.commandBits < 4096) this.commandBits += 1
    if (state !== 'pause-dr' || previous !== 'exit1-dr') return
    if (!this.commandLevel) {
      if (this.commandBits === 0) {
        if (this.zeroScans < 2) this.zeroScans += 1
      } else if (this.zeroScans === 2 && this.commandBits === 1) {
        this.commandLevel = true
        this.commandStep = 0
      } else this.zeroScans = 0
      return
    }
    if (this.commandStep === 0) {
      this.commandOpcode = this.commandBits
      this.commandStep = 1
    } else if (this.commandStep === 1) {
      this.commandStep = this.commandBits === 0 ? 2 : 0
    } else if (this.commandStep === 2) {
      this.commandParameter = this.commandBits
      this.commandStep = 3
    } else {
      if (
        this.commandBits === 0 &&
        this.commandOpcode === 3 &&
        (this.commandParameter === 8 || this.commandParameter === 9)
      ) {
        this.activeFormat = this.commandParameter === 8 ? 'oscan0' : 'oscan1'
        this.checking = true
        this.checkBits = 0
        this.checkValue = 0
      }
      this.commandStep = 0
    }
  }
  async clock(tms: bool, input: u8, output: u8, start: Tick, tick: Tick): Promise<void> {
    const previous = this.state
    const state = next(previous, tms)
    if (!this.active) this.command(state, previous)
    const resetAlready = this.ones === 5
    if (tms) {
      if (this.ones < 5) this.ones += 1
    } else this.ones = 0
    if (this.ones === 5 && !resetAlready) {
      await this.cut(tick)
      await this.out.emit({ kind: 'reset', by: 'tms' }, one(tick))
      // A continuous reset does not create repeated reset events.
      this.ones = 5
      return
    }
    const scan = this.scan
    if ((previous === 'shift-ir' || previous === 'shift-dr') && scan !== undefined) {
      const tdi: Bounded<u8, 32> = [input]
      const tdo: Bounded<u8, 32> = [output]
      await this.out.emit(
        { kind: 'shift', scan, offset: this.bits, tdi, tdo, bits: 1 },
        over(start, tick + 1, tick),
      )
      this.bits += 1
    }
    this.state = state
    if (state !== previous) await this.out.emit({ kind: 'state', state }, one(tick))
    if ((state === 'shift-ir' || state === 'shift-dr') && this.scan === undefined && this.active) {
      const register: Register = state === 'shift-ir' ? 'ir' : 'dr'
      const scanId = this.nextScan
      this.scan = scanId
      this.nextScan += 1
      this.bits = 0
      await this.out.emit({ kind: 'scan-begin', scan: scanId, register }, one(tick))
    }
    if ((state === 'update-ir' || state === 'update-dr') && scan !== undefined) {
      await this.out.emit(
        { kind: 'scan-end', scan, bits: this.bits, reason: 'complete' },
        one(tick),
      )
      this.scan = undefined
    }
  }
  override async run(): Promise<void> {
    let levels = await this.signal.sample()
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      const clock = edge.levels.high(this.config.clock)
      const oldClock = levels.high(this.config.clock)
      const data = edge.levels.high(this.config.data)
      if (clock && oldClock && data !== levels.high(this.config.data)) {
        if (this.escapes < 8) this.escapes += 1
      }
      if (!clock && oldClock) {
        if (this.escapes >= 6) {
          await this.cut(edge.tick)
          this.active = false
          this.selecting = this.escapes < 8
          this.selection = 0
          this.selectionBits = 0
          this.selectionLength = 12
          this.commandLevel = false
          this.zeroScans = 0
          this.checking = false
        }
        this.escapes = 0
      }
      levels = edge.levels
      if (!clock || oldClock) continue
      if (this.selecting) {
        this.selection |= (fromBool(data) as u64) << (this.selectionBits as u64)
        this.selectionBits += 1
        if (this.selectionBits === 8 && this.selection === 0) this.selectionLength = 36
        if (this.selectionBits === this.selectionLength) {
          // OAC=0xC, EC=0x8, CP=0: explicitly documented short-form boot.
          this.active = this.selectionLength === 12 && this.selection === 0x08c
          this.activeFormat = 'oscan1'
          this.selecting = false
          this.phase = 0
          this.state = 'run-test-idle'
          this.ones = 0
        }
        continue
      }
      if (this.checking) {
        this.checkValue |= fromBool(data) << this.checkBits
        this.checkBits += 1
        if (this.checkBits === 4) {
          this.active = this.checkValue === 0
          this.checking = false
          this.phase = 0
        }
        continue
      }
      if (!this.active) {
        await this.clock(data, 0, 0, edge.tick, edge.tick)
        continue
      }
      if (this.phase === 0) {
        this.tdiBit = data ? 0 : 1
        this.first = edge.tick
        this.ready = 0
        this.stalls = 0
        this.phase = 1
      } else if (this.phase === 1) {
        this.tms = data
        this.phase = 2
      } else if (this.phase === 2 && this.activeFormat === 'oscan0') {
        this.stalls += 1
        if (data) this.ready += 1
        else this.ready = 0
        if (this.ready === this.config.readyOnes) this.phase = 3
        else if (this.stalls === this.config.maxStallCycles) {
          await this.cut(edge.tick)
          this.active = false
        }
      } else if (this.phase === 4) {
        this.delays += 1
        if (this.delays === this.config.delayCycles) {
          const first = this.first
          await this.clock(this.tms, this.tdiBit, this.tdoBit, first ?? edge.tick, edge.tick)
          this.first = undefined
          this.phase = 0
        }
      } else {
        if (this.config.delayCycles === 0) {
          const first = this.first
          await this.clock(this.tms, this.tdiBit, fromBool(data), first ?? edge.tick, edge.tick)
          this.first = undefined
          this.phase = 0
        } else {
          this.tdoBit = fromBool(data)
          this.delays = 0
          this.phase = 4
        }
      }
    }
  }
}
export const cjtag = defineLogicDecoder({
  name: 'cjtag',
  category: 'bus',
  config: {
    defaults: {
      clock: 0,
      data: 1,
      startAligned: false,
      format: 'oscan1',
      readyOnes: 1,
      delayCycles: 0,
      maxStallCycles: 4096,
    },
    validate,
  },
  requires: (config) => ({ channels: atLeast(max(config.clock, config.data) + 1) }),
  output: jtagEvents,
  props: () => ({ tdi: true, tdo: true }),
  locales: {
    'en-US': {
      name: 'cJTAG OScan0/1',
      summary: 'Two-wire OScan0/1 with bounded ready stalls and observed activation',
      fields: {
        clock: 'TCKC',
        data: 'TMSC',
        startAligned: 'Capture starts at a known nTDI phase',
        format: 'Initial scan format',
        readyOnes: 'Consecutive ready bits',
        delayCycles: 'Fixed delay clocks',
        maxStallCycles: 'Maximum ready clocks',
      },
      events: {
        state: 'TAP state',
        reset: 'Reset',
        'scan-begin': 'Scan begin',
        shift: 'Shift bit',
        'scan-end': 'Scan end',
      },
      eventFields: {
        state: 'State',
        by: 'Reset source',
        scan: 'Scan',
        register: 'Register',
        offset: 'Bit offset',
        tdi: 'TDI bits',
        tdo: 'TDO bits',
        bits: 'Bits',
        reason: 'End reason',
      },
    },
    'zh-CN': {
      name: 'cJTAG OScan0/1',
      summary: '双线OScan0/1扫描，支持有界就绪等待与观测激活',
      fields: {
        clock: 'TCKC',
        data: 'TMSC',
        startAligned: '采集从已知nTDI相位开始',
        format: '初始扫描格式',
        readyOnes: '连续就绪位数',
        delayCycles: '固定延迟周期',
        maxStallCycles: '最大就绪周期',
      },
      events: {
        state: 'TAP状态',
        reset: '复位',
        'scan-begin': '扫描开始',
        shift: '移位位',
        'scan-end': '扫描结束',
      },
      eventFields: {
        state: '状态',
        by: '复位源',
        scan: '扫描',
        register: '寄存器',
        offset: '位偏移',
        tdi: 'TDI位',
        tdo: 'TDO位',
        bits: '位数',
        reason: '结束原因',
      },
    },
  },
  decoder: Cjtag,
})
