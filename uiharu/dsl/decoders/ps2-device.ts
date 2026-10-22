// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Integer assertions select IR widths and truncation; TypeScript aliases alone do not encode these semantics. */
/** Original PS/2 Set 2 and standard three-byte mouse consumer. Wire facts:
 * Microchip AVR313/AN1235; Holtek HT82K629B keyboard command table;
 * Infineon PS2D software module datasheet. No keyboard layout is inferred. */
import type { Provenance, Range, Tick, bool, u8, u16 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
import { ps2Events, type Ps2Event } from '../contracts/ps2'
export type Mode =
  'keyboard-set1' | 'keyboard-set2' | 'keyboard-set3' | 'mouse' | 'mouse-wheel' | 'mouse-explorer'
export interface Config {
  mode: Mode
}
export type Reason =
  | 'input-error'
  | 'aborted'
  | 'bad-sequence'
  | 'unsupported-set'
  | 'mouse-sync'
  | 'host-unacknowledged'
export type Event =
  | { kind: 'key'; code: u8; extended: bool; released: bool }
  | { kind: 'pause' }
  | { kind: 'print-screen'; released: bool }
  | {
      kind: 'mouse'
      buttons: u8
      x: u16
      xNegative: bool
      y: u16
      yNegative: bool
      xOverflow: bool
      yOverflow: bool
    }
  | { kind: 'wheel'; buttons: u8; magnitude: u8; negative: bool }
  | { kind: 'command'; value: u8; parameter: bool }
  | { kind: 'reply'; command: u8 | undefined; index: u8; value: u8 }
  | { kind: 'status'; value: u8 }
  | { kind: 'error'; reason: Reason }
export const events = eventSchema<Event>('ps2-device.events', 1).roles({
  key: 'data',
  pause: 'data',
  'print-screen': 'data',
  mouse: 'data',
  wheel: 'data',
  command: 'command',
  reply: 'control',
  status: 'line',
  error: 'error',
})
function parameterCommand(b: u8, mode: Mode): bool {
  return b === 0xf3 || (isMouse(mode) ? b === 0xe8 : b === 0xed || b === 0xf0)
}
function isMouse(mode: Mode): bool {
  return mode === 'mouse' || mode === 'mouse-wheel' || mode === 'mouse-explorer'
}
function scanSet(mode: Mode): u8 {
  if (mode === 'keyboard-set1') return 1
  if (mode === 'keyboard-set3') return 3
  return 2
}
function pauseByte(index: u8): u8 {
  if (index === 1) return 0x14
  if (index === 2) return 0x77
  if (index === 3) return 0xe1
  if (index === 4) return 0xf0
  if (index === 5) return 0x14
  if (index === 6) return 0xf0
  return 0x77
}
class Ps2Device extends StreamDecoder<Config, Ps2Event, Event> {
  first: Provenance | undefined = undefined
  extended = false
  released = false
  pause: u8 = 0
  print: u8 = 0
  mouseCount: u8 = 0
  header: u8 = 0
  x: u8 = 0
  y: u8 = 0
  set2 = true
  command: u8 | undefined = undefined
  needAck = false
  needParameter = false
  parameter: u8 | undefined = undefined
  replies: u8 = 0
  replyIndex: u8 = 0
  resetId = false
  override hold(): Tick | undefined {
    const first = this.first
    if (first !== undefined) return first.span.start
    return undefined
  }
  clearPacket(): void {
    this.first = undefined
    this.extended = false
    this.released = false
    this.pause = 0
    this.print = 0
    this.mouseCount = 0
  }
  clearCommand(): void {
    this.command = undefined
    this.needAck = false
    this.needParameter = false
    this.parameter = undefined
    this.replies = 0
    this.replyIndex = 0
    this.resetId = false
  }
  async fail(reason: Reason, at: Provenance): Promise<void> {
    const first = this.first
    let placed = at
    if (first !== undefined) placed = over(first.span.start, at.span.end, at.settled)
    await this.out.emit({ kind: 'error', reason }, placed)
    this.clearPacket()
  }
  override async onGap(range: Range): Promise<void> {
    if (this.first !== undefined || this.command !== undefined)
      await this.fail('aborted', over(range.start, range.start, range.start))
    this.clearPacket()
    this.clearCommand()
  }
  override async onEnd(tick: Tick): Promise<void> {
    if (this.first !== undefined || this.command !== undefined)
      await this.fail('aborted', over(tick, tick, tick))
    this.clearPacket()
    this.clearCommand()
  }
  async host(b: u8, at: Provenance): Promise<void> {
    if (this.first !== undefined) await this.fail('aborted', at)
    if (this.needParameter && !this.needAck) {
      this.parameter = b
      this.needParameter = false
      this.needAck = true
      await this.out.emit({ kind: 'command', value: b, parameter: true }, at)
      return
    }
    this.clearCommand()
    this.command = b
    this.needAck = b !== 0xee
    if (b === 0xee) this.replies = 1
    await this.out.emit({ kind: 'command', value: b, parameter: false }, at)
  }
  async control(b: u8, at: Provenance): Promise<bool> {
    const command = this.command
    if (command === undefined) return false
    if (this.needAck) {
      await this.out.emit({ kind: 'reply', command, index: 0, value: b }, at)
      if (b === 0xfe) return true
      if (b !== 0xfa) {
        this.clearCommand()
        return true
      }
      this.needAck = false
      const parameter = this.parameter
      if (parameter === undefined && parameterCommand(command, this.config.mode)) {
        this.needParameter = true
        return true
      }
      if (command === 0xf0 && parameter !== undefined && !isMouse(this.config.mode)) {
        if (parameter === 0) this.replies = 1
        else {
          this.set2 = parameter === scanSet(this.config.mode)
          if (!this.set2) await this.fail('unsupported-set', at)
        }
      } else if (command === 0xf2) this.replies = isMouse(this.config.mode) ? 1 : 2
      else if (command === 0xff) {
        this.replies = 1
        this.resetId = isMouse(this.config.mode)
        this.set2 = scanSet(this.config.mode) === 2
      } else if (command === 0xe9 && isMouse(this.config.mode)) this.replies = 3
      else if (command === 0xeb && isMouse(this.config.mode)) {
        this.clearCommand()
        return true
      }
      if (this.replies === 0) this.clearCommand()
      return true
    }
    if (this.replies !== 0) {
      await this.out.emit({ kind: 'reply', command, index: this.replyIndex, value: b }, at)
      this.replyIndex += 1
      this.replies -= 1
      if (command === 0xf0 && !isMouse(this.config.mode))
        this.set2 =
          b === scanSet(this.config.mode) ||
          (scanSet(this.config.mode) === 1 && b === 0x43) ||
          (scanSet(this.config.mode) === 2 && b === 0x41) ||
          (scanSet(this.config.mode) === 3 && b === 0x3f)
      if (this.replies === 0) {
        if (this.resetId && b === 0xaa) {
          this.resetId = false
          this.replies = 1
        } else this.clearCommand()
      }
      return true
    }
    return false
  }
  async keyboard(b: u8, at: Provenance): Promise<void> {
    if (!this.set2) {
      await this.fail('unsupported-set', at)
      return
    }
    if (this.config.mode === 'keyboard-set1') {
      await this.keyboardSet1(b, at)
      return
    }
    if (this.config.mode === 'keyboard-set3') {
      if (b === 0xf0) {
        if (this.released) await this.fail('bad-sequence', at)
        else {
          this.released = true
          this.first = at
        }
        return
      }
      const first = this.first
      await this.out.emit(
        { kind: 'key', code: b, extended: false, released: this.released },
        first === undefined ? at : over(first.span.start, at.span.end, at.settled),
      )
      this.clearPacket()
      return
    }
    if (this.pause !== 0) {
      if (b !== pauseByte(this.pause)) {
        await this.fail('bad-sequence', at)
        return
      }
      this.pause += 1
      if (this.pause === 8) {
        const first = this.first
        if (first !== undefined)
          await this.out.emit({ kind: 'pause' }, over(first.span.start, at.span.end, at.settled))
        this.clearPacket()
      }
      return
    }
    if (this.print !== 0) {
      let expected: u8 = 0xe0
      if (this.print === 2) expected = 0x7c
      if (this.print === 4) expected = 0xf0
      if (this.print === 5) expected = 0x12
      if (b !== expected) {
        await this.fail('bad-sequence', at)
        return
      }
      if (this.print === 2 || this.print === 5) {
        const first = this.first
        if (first !== undefined)
          await this.out.emit(
            { kind: 'print-screen', released: this.print === 5 },
            over(first.span.start, at.span.end, at.settled),
          )
        this.clearPacket()
      } else this.print += 1
      return
    }
    if (b === 0xe1) {
      if (this.first !== undefined) {
        await this.fail('bad-sequence', at)
        return
      }
      this.first = at
      this.pause = 1
      return
    }
    if (b === 0xe0) {
      if (this.extended) {
        await this.fail('bad-sequence', at)
        return
      }
      if (this.first === undefined) this.first = at
      this.extended = true
      return
    }
    if (b === 0xf0) {
      if (this.released) {
        await this.fail('bad-sequence', at)
        return
      }
      if (this.first === undefined) this.first = at
      this.released = true
      return
    }
    if (this.extended && b === 0x12 && !this.released) {
      this.print = 1
      return
    }
    if (this.extended && b === 0x7c && this.released) {
      this.print = 3
      return
    }
    const first = this.first
    let placed = at
    if (first !== undefined) placed = over(first.span.start, at.span.end, at.settled)
    await this.out.emit(
      { kind: 'key', code: b, extended: this.extended, released: this.released },
      placed,
    )
    this.clearPacket()
  }
  async keyboardSet1(b: u8, at: Provenance): Promise<void> {
    if (this.pause !== 0) {
      let expected: u8 = 0xc5
      if (this.pause === 1) expected = 0x1d
      if (this.pause === 2) expected = 0x45
      if (this.pause === 3) expected = 0xe1
      if (this.pause === 4) expected = 0x9d
      if (b !== expected) {
        await this.fail('bad-sequence', at)
        return
      }
      this.pause += 1
      if (this.pause === 6) {
        const first = this.first
        if (first !== undefined)
          await this.out.emit({ kind: 'pause' }, over(first.span.start, at.span.end, at.settled))
        this.clearPacket()
      }
      return
    }
    if (this.print !== 0) {
      let expected: u8 = 0xe0
      if (this.print === 2) expected = this.released ? 0xaa : 0x37
      if (b !== expected) {
        await this.fail('bad-sequence', at)
        return
      }
      this.print += 1
      if (this.print === 3) {
        const first = this.first
        if (first !== undefined)
          await this.out.emit(
            { kind: 'print-screen', released: this.released },
            over(first.span.start, at.span.end, at.settled),
          )
        this.clearPacket()
      }
      return
    }
    if (b === 0xe1) {
      this.first = at
      this.pause = 1
      return
    }
    if (b === 0xe0) {
      if (this.extended) {
        await this.fail('bad-sequence', at)
        return
      }
      this.first = at
      this.extended = true
      return
    }
    if (this.extended && (b === 0x2a || b === 0xb7)) {
      this.released = b === 0xb7
      this.print = 1
      return
    }
    const first = this.first
    await this.out.emit(
      { kind: 'key', code: b & 0x7f, extended: this.extended, released: (b & 0x80) !== 0 },
      first === undefined ? at : over(first.span.start, at.span.end, at.settled),
    )
    this.clearPacket()
  }
  async mouse(b: u8, at: Provenance): Promise<void> {
    if (this.mouseCount === 0) {
      if ((b & 8) === 0) {
        await this.fail('mouse-sync', at)
        return
      }
      this.header = b
      this.first = at
      this.mouseCount = 1
      return
    }
    if (this.mouseCount === 1) {
      this.x = b
      this.mouseCount = 2
      return
    }
    if (this.mouseCount === 2 && this.config.mode !== 'mouse') {
      this.y = b
      this.mouseCount = 3
      return
    }
    const yByte: u8 = this.config.mode === 'mouse' ? b : this.y
    const first = this.first
    const xn = (this.header & 16) !== 0
    const yn = (this.header & 32) !== 0
    const x: u16 = xn ? 256 - (this.x as u16) : this.x
    const y: u16 = yn ? 256 - (yByte as u16) : yByte
    if (first !== undefined)
      await this.out.emit(
        {
          kind: 'mouse',
          buttons: this.header & 7,
          x,
          xNegative: xn && x !== 0,
          y,
          yNegative: yn && y !== 0,
          xOverflow: (this.header & 64) !== 0,
          yOverflow: (this.header & 128) !== 0,
        },
        over(first.span.start, at.span.end, at.settled),
      )
    if (this.config.mode !== 'mouse') {
      const explorer = this.config.mode === 'mouse-explorer'
      const wheel: u8 = explorer ? b & 15 : b
      const negative = explorer ? (wheel & 8) !== 0 : (wheel & 128) !== 0
      const modulus: u16 = explorer ? 16 : 256
      const magnitude: u8 = negative ? ((modulus - wheel) as u8) : wheel
      if (explorer && (b & 0xc0) !== 0) await this.fail('bad-sequence', at)
      else
        await this.out.emit(
          {
            kind: 'wheel',
            buttons: (this.header & 7) | (explorer ? (b & 0x30) >> 1 : 0),
            magnitude,
            negative,
          },
          at,
        )
    }
    this.clearPacket()
  }
  override async run(): Promise<void> {
    this.set2 = true
    for (;;) {
      const received = await this.input.next()
      const event = received.event
      const at = received.at
      if (event.kind === 'error') {
        await this.fail('input-error', at)
        this.clearCommand()
        continue
      }
      if (event.kind === 'frame') {
        if (!event.parityOk || !event.stopOk) {
          await this.fail('input-error', at)
          this.clearCommand()
          continue
        }
        if (event.direction === 'host-to-device') {
          if (event.ack !== true) {
            await this.fail('host-unacknowledged', at)
            this.clearCommand()
          } else await this.host(event.value, at)
          continue
        }
        const b = event.value
        if (await this.control(b, at)) continue
        if (isMouse(this.config.mode)) {
          await this.mouse(b, at)
          continue
        }
        if (
          this.first === undefined &&
          (b === 0xfa ||
            b === 0xfe ||
            // Set1 AA is also Left Shift break; explicit replies were handled above.
            (b === 0xaa && this.config.mode !== 'keyboard-set1') ||
            b === 0xfc ||
            b === 0xfd ||
            b === 0 ||
            b === 0xff)
        ) {
          if (this.first !== undefined) await this.fail('aborted', at)
          await this.out.emit({ kind: 'status', value: b }, at)
          continue
        }
        await this.keyboard(b, at)
      }
    }
  }
}
export const ps2Device = defineStreamDecoder({
  name: 'ps2-device',
  category: 'device',
  config: { defaults: { mode: 'keyboard-set2' } },
  input: ps2Events,
  requires: () => ({ wordBits: 8, bitOrder: 'lsb-first' }),
  output: events,
  locales: {
    'en-US': {
      name: 'PS/2 keyboard / mouse',
      summary: 'Set 2 make/break, Pause/Print Screen, standard mouse packets and host replies',
      fields: { mode: 'Device profile' },
      values: {
        mode: {
          'keyboard-set1': 'Keyboard scan set 1',
          'keyboard-set2': 'Keyboard scan set 2',
          'keyboard-set3': 'Keyboard scan set 3',
          mouse: 'Standard mouse',
          'mouse-wheel': 'Three-button wheel mouse',
          'mouse-explorer': 'Five-button wheel mouse',
        },
      },
      events: {
        key: 'Key scan code',
        pause: 'Pause',
        'print-screen': 'Print Screen',
        mouse: 'Mouse movement',
        wheel: 'Wheel and buttons',
        command: 'Host command',
        reply: 'Device reply',
        status: 'Device status',
        error: 'Protocol error',
      },
      eventFields: {
        magnitude: 'Wheel magnitude',
        negative: 'Negative wheel',
        code: 'Scan code',
        extended: 'Extended',
        released: 'Released',
        buttons: 'Button mask',
        x: 'X magnitude',
        xNegative: 'Negative X',
        y: 'Y magnitude',
        yNegative: 'Negative Y',
        xOverflow: 'X overflow',
        yOverflow: 'Y overflow',
        value: 'Value',
        parameter: 'Command parameter',
        command: 'Related host command',
        index: 'Reply index',
        reason: 'Error reason',
      },
    },
    'zh-CN': {
      name: 'PS/2键盘/鼠标',
      summary: '扫描码集2通断码、Pause/Print Screen、标准鼠标包和主机应答',
      fields: { mode: '设备格式' },
      values: {
        mode: {
          'keyboard-set1': '键盘扫描码集1',
          'keyboard-set2': '键盘扫描码集2',
          'keyboard-set3': '键盘扫描码集3',
          mouse: '标准鼠标',
          'mouse-wheel': '三键滚轮鼠标',
          'mouse-explorer': '五键滚轮鼠标',
        },
      },
      events: {
        key: '按键扫描码',
        pause: 'Pause',
        'print-screen': 'Print Screen',
        mouse: '鼠标移动',
        wheel: '滚轮和按键',
        command: '主机命令',
        reply: '设备应答',
        status: '设备状态',
        error: '协议错误',
      },
      eventFields: {
        magnitude: '滚轮绝对值',
        negative: '滚轮负方向',
        code: '扫描码',
        extended: '扩展码',
        released: '释放',
        buttons: '按键掩码',
        x: 'X绝对值',
        xNegative: 'X负方向',
        y: 'Y绝对值',
        yNegative: 'Y负方向',
        xOverflow: 'X溢出',
        yOverflow: 'Y溢出',
        value: '数值',
        parameter: '命令参数',
        command: '关联主机命令',
        index: '应答索引',
        reason: '错误原因',
      },
    },
  },
  decoder: Ps2Device,
})
