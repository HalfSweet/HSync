// SPDX-License-Identifier: Apache-2.0
/** AVR XMEGA PDI: Microchip XMEGA A manual8077I §§29.3–29.6 and AVR1612.
 * Independent synchronous character/instruction parser. Fixed state, no repeat buffer. */
import type { Channel, Range, Tick, Provenance, bool, u8, u32, u64 } from 'uiharu'
import {
  LogicDecoder,
  defineLogicDecoder,
  eventSchema,
  mask,
  max,
  atLeast,
  one,
  over,
  join,
  fromBool,
} from 'uiharu'
export interface Config {
  clock: Channel
  data: Channel
  startAligned: bool
}
export type Command = 'lds' | 'ld' | 'sts' | 'st' | 'ldcs' | 'repeat' | 'stcs' | 'key'
export type Role = 'address' | 'read-data' | 'write-data' | 'key' | 'repeat-count'
export type End = 'complete' | 'aborted' | 'invalid-opcode'
export type Event =
  | { kind: 'character'; value: u8; parityOk: bool; stopOk: bool }
  | { kind: 'break' }
  | {
      kind: 'instruction'
      op: u64
      command: Command
      opcode: u8
      repeat: u64
      pointerMode: u8
      controlRegister: u8
    }
  | { kind: 'operand-byte'; op: u64; iteration: u64; index: u8; role: Role; value: u8 }
  | { kind: 'value'; op: u64; iteration: u64; role: Role; value: u32; address: u32 | undefined }
  | { kind: 'end'; op: u64; reason: End }
export const events = eventSchema<Event>('avr-pdi.events', 1).roles({
  character: { role: 'data', errorWhen: [{ parityOk: false }, { stopOk: false }] },
  break: 'control',
  instruction: 'command',
  'operand-byte': 'data',
  value: 'data',
  end: 'framing',
})
function command(code: u8): Command {
  switch (code) {
    case 0:
      return 'lds'
    case 1:
      return 'ld'
    case 2:
      return 'sts'
    case 3:
      return 'st'
    case 4:
      return 'ldcs'
    case 5:
      return 'repeat'
    case 6:
      return 'stcs'
    default:
      return 'key'
  }
}
function validate(c: Config): string | undefined {
  if (c.clock >= 64 || c.data >= 64 || c.clock === c.data) return 'avr-pdi.config.channels'
  return undefined
}
class AvrPdi extends LogicDecoder<Config, Event> {
  readonly watch = mask(this.config.clock)
  frameBits: u8 = 0
  frameValue: u8 = 0
  parity: u8 = 0
  parityOk = true
  characterFirst: Provenance | undefined = undefined
  firstLow: Tick | undefined = undefined
  lowRun: u8 = 0
  highRun: u8 = 0
  aligned = false
  open = false
  valueFirst: Provenance | undefined = undefined
  op: u64 = 0
  code: u8 = 0
  opcode: u8 = 0
  width: u8 = 0
  addressWidth: u8 = 0
  addressPhase = false
  pointerMode: u8 = 0
  count: u8 = 0
  assembled: u32 = 0
  iterations: u64 = 1
  iteration: u64 = 0
  pendingRepeat: u64 = 1
  direct: u32 | undefined = undefined
  directRegister: u32 | undefined = undefined
  commandAligned = this.config.startAligned
  pointer: u32 | undefined = undefined
  override hold(): Tick | undefined {
    const value = this.valueFirst
    const character = this.characterFirst
    const low = this.firstLow
    if (value !== undefined) {
      if (low !== undefined && low < value.span.start) return low
      return value.span.start
    }
    if (character !== undefined) {
      if (low !== undefined && low < character.span.start) return low
      return character.span.start
    }
    return low
  }
  override async onGap(range: Range): Promise<void> {
    await this.cut(range.start)
    this.aligned = false
    this.lowRun = 0
    this.firstLow = undefined
    this.highRun = 0
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
    this.firstLow = undefined
  }
  async cut(tick: Tick): Promise<void> {
    if (this.open)
      await this.out.emit({ kind: 'end', op: this.op, reason: 'aborted' }, over(tick, tick, tick))
    this.open = false
    this.valueFirst = undefined
    this.characterFirst = undefined
    this.frameBits = 0
    this.pendingRepeat = 1
    this.pointer = undefined
    this.directRegister = undefined
    this.commandAligned = false
  }
  async done(at: Provenance): Promise<void> {
    await this.out.emit({ kind: 'end', op: this.op, reason: 'complete' }, at)
    this.open = false
    this.valueFirst = undefined
  }
  async byte(byte: u8, at: Provenance): Promise<void> {
    if (!this.open) {
      this.op += 1
      this.open = true
      this.code = byte >> 5
      this.opcode = byte
      this.pointerMode = (byte >> 2) & 3
      this.width = (byte & 3) + 1
      this.addressWidth = ((byte >> 2) & 3) + 1
      this.addressPhase = this.code === 0 || this.code === 2
      this.count = 0
      this.assembled = 0
      this.iteration = 0
      this.direct = undefined
      this.iterations = this.code === 5 || this.code === 7 ? 1 : this.pendingRepeat
      this.pendingRepeat = 1
      if (this.code === 4 || this.code === 6) this.width = 1
      if (this.code === 7) this.width = 8
      await this.out.emit(
        {
          kind: 'instruction',
          op: this.op,
          command: command(this.code),
          opcode: byte,
          repeat: this.iterations,
          pointerMode: this.pointerMode,
          controlRegister: byte & 15,
        },
        at,
      )
      if (
        (byte & 16) !== 0 ||
        ((this.code === 1 || this.code === 3) && this.pointerMode === 3) ||
        (this.code === 5 && (byte & 12) !== 0) ||
        (this.code === 7 && (byte & 15) !== 0)
      ) {
        await this.out.emit({ kind: 'end', op: this.op, reason: 'invalid-opcode' }, at)
        this.open = false
        this.pointer = undefined
        this.aligned = false
        this.commandAligned = false
      }
      return
    }
    let role: Role = 'write-data'
    if (this.addressPhase) role = 'address'
    else if (this.code === 0 || this.code === 1 || this.code === 4) role = 'read-data'
    else if (this.code === 5) role = 'repeat-count'
    else if (this.code === 7) role = 'key'
    if (this.count === 0) this.valueFirst = at
    await this.out.emit(
      {
        kind: 'operand-byte',
        op: this.op,
        iteration: this.iteration,
        index: this.count,
        role,
        value: byte,
      },
      at,
    )
    if (this.count < 4) this.assembled |= (byte as u32) << ((this.count as u64) * 8)
    this.count += 1
    const width = this.addressPhase ? this.addressWidth : this.width
    if (this.count !== width) return
    const start = this.valueFirst
    const placed = start === undefined ? at : join(start, at)
    if (this.addressPhase) {
      if (this.addressWidth === 4) this.directRegister = this.assembled
      else {
        const previous = this.directRegister
        if (previous !== undefined) {
          const lowMask: u32 = (1 << ((this.addressWidth as u64) * 8)) - 1
          this.directRegister = (previous & ~lowMask) | this.assembled
        }
      }
      this.direct = this.directRegister
      await this.out.emit(
        {
          kind: 'value',
          op: this.op,
          iteration: this.iteration,
          role,
          value: this.assembled,
          address: this.direct,
        },
        placed,
      )
      this.addressPhase = false
      this.valueFirst = undefined
      this.count = 0
      this.assembled = 0
      return
    }
    if (this.code !== 7) {
      let address: u32 | undefined = undefined
      if (this.code === 0 || this.code === 2) address = this.direct
      else if ((this.code === 1 || this.code === 3) && this.pointerMode <= 1) address = this.pointer
      await this.out.emit(
        {
          kind: 'value',
          op: this.op,
          iteration: this.iteration,
          role,
          value: this.assembled,
          address,
        },
        placed,
      )
    }
    if (this.code === 5) this.pendingRepeat = (this.assembled as u64) + 1
    if ((this.code === 1 || this.code === 3) && this.pointerMode === 2) {
      if (this.width === 4) this.pointer = this.assembled
      else {
        const pointer = this.pointer
        if (pointer !== undefined) {
          const lowMask: u32 = (1 << ((this.width as u64) * 8)) - 1
          this.pointer = (pointer & ~lowMask) | this.assembled
        }
      }
    }
    if ((this.code === 1 || this.code === 3) && this.pointerMode === 1) {
      const pointer = this.pointer
      if (pointer !== undefined) this.pointer = pointer + this.width
    }
    this.iteration += 1
    if (this.iteration === this.iterations) {
      await this.done(at)
      return
    }
    this.addressPhase = this.code === 0 || this.code === 2
    this.count = 0
    this.assembled = 0
    this.valueFirst = undefined
  }
  override async run(): Promise<void> {
    await this.signal.sample()
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      if (!edge.levels.high(this.config.clock)) continue
      const bit: u8 = fromBool(edge.levels.high(this.config.data))
      const at = one(edge.tick)
      if (bit === 0) {
        if (this.lowRun === 0) this.firstLow = edge.tick
        this.highRun = 0
        if (this.lowRun < 12) this.lowRun += 1
        if (this.lowRun === 12) {
          await this.cut(edge.tick)
          const low = this.firstLow
          await this.out.emit(
            { kind: 'break' },
            low === undefined ? at : over(low, edge.tick + 1, edge.tick),
          )
          this.commandAligned = true
          this.firstLow = undefined
          this.lowRun = 13
          this.aligned = false
          continue
        }
      } else {
        this.lowRun = 0
        this.firstLow = undefined
        if (this.highRun < 2) this.highRun += 1
        if (!this.aligned && this.highRun === 2) this.aligned = true
      }
      if (!this.aligned) continue
      if (this.frameBits === 0) {
        if (bit === 0) {
          this.frameBits = 1
          this.frameValue = 0
          this.parity = 0
          this.characterFirst = at
        }
        continue
      }
      if (this.frameBits <= 8) {
        this.frameValue |= bit << ((this.frameBits - 1) as u64)
        this.parity ^= bit
        this.frameBits += 1
        continue
      }
      if (this.frameBits === 9) {
        this.parityOk = bit === this.parity
        this.frameBits = 10
        continue
      }
      const first = this.characterFirst
      const placed = first === undefined ? at : join(first, at)
      await this.out.emit(
        { kind: 'character', value: this.frameValue, parityOk: this.parityOk, stopOk: bit !== 0 },
        placed,
      )
      this.frameBits = 0
      this.characterFirst = undefined
      if (!this.parityOk || bit === 0) {
        await this.cut(edge.tick)
        this.aligned = false
        continue
      }
      if (this.commandAligned) await this.byte(this.frameValue, placed)
    }
  }
}
export const avrPdi = defineLogicDecoder({
  name: 'avr-pdi',
  category: 'bus',
  config: { defaults: { clock: 0, data: 1, startAligned: false }, validate },
  requires: (config) => ({ channels: atLeast(max(config.clock, config.data) + 1) }),
  output: events,
  props: () => ({ physical: 'synchronous-pdi', instructions: true }),
  locales: {
    'en-US': {
      name: 'AVR XMEGA PDI',
      summary: 'Synchronous PDI characters, all instructions and streamed repeats',
      fields: {
        clock: 'PDI_CLK',
        data: 'PDI_DATA',
        startAligned: 'Start at a known instruction boundary',
      },
      events: {
        character: 'Character',
        break: 'BREAK',
        instruction: 'Instruction',
        'operand-byte': 'Operand byte',
        value: 'Assembled value',
        end: 'End',
      },
      eventFields: {
        value: 'Value',
        parityOk: 'Even parity OK',
        stopOk: 'Stop bit OK',
        op: 'Operation',
        command: 'Instruction',
        opcode: 'Opcode',
        repeat: 'Execution count',
        pointerMode: 'Pointer mode',
        controlRegister: 'Control register',
        iteration: 'Iteration',
        index: 'Byte index',
        role: 'Operand role',
        address: 'Known address',
        reason: 'End reason',
      },
    },
    'zh-CN': {
      name: 'AVR XMEGA PDI',
      summary: '同步PDI字符、完整指令集及流式重复操作',
      fields: { clock: 'PDI_CLK', data: 'PDI_DATA', startAligned: '从已知指令边界开始' },
      events: {
        character: '字符',
        break: 'BREAK',
        instruction: '指令',
        'operand-byte': '操作数字节',
        value: '组合数值',
        end: '结束',
      },
      eventFields: {
        value: '数值',
        parityOk: '偶校验正确',
        stopOk: '停止位正确',
        op: '操作号',
        command: '指令',
        opcode: '操作码',
        repeat: '执行次数',
        pointerMode: '指针模式',
        controlRegister: '控制寄存器',
        iteration: '重复编号',
        index: '字节索引',
        role: '操作数用途',
        address: '已知地址',
        reason: '结束原因',
      },
    },
  },
  decoder: AvrPdi,
})
