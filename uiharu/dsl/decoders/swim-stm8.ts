// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Assertions preserve IR integer widths. */
/** Independently authored from ST UM0470 Rev4 §§3.2–3.6; native decoder retained as baseline. */
import type { Channel, Tick, Range, u8, u32, u64, bool } from 'uiharu'
import {
  LogicDecoder,
  defineLogicDecoder,
  eventSchema,
  mask,
  over,
  atLeast,
  mulDiv,
  mulDivCeil,
} from 'uiharu'
export type Direction = 'host-to-target' | 'target-to-host'
export type Field =
  'count' | 'address-extended' | 'address-high' | 'address-low' | 'data' | 'unclassified'
export type ErrorReason = 'aborted' | 'reset' | 'invalid-count'
export type Speed = 'low' | 'high'
export type SyncAfter = 'entry' | 'reset'
export interface SwimStm8Config {
  data: Channel
  clockHz: u32
  speed: Speed
  startActive: bool
  calibrateSync: bool
}
export type SwimEvent =
  | { kind: 'command'; value: u8; parityOk: bool; ack: bool }
  | {
      kind: 'byte'
      direction: Direction
      field: Field
      value: u8
      parityOk: bool
      ack: bool
      address: u32 | undefined
    }
  | { kind: 'reset' }
  | { kind: 'entry' }
  | { kind: 'calibration'; clockHz: u32; lowTicks: u64; after: SyncAfter }
  | { kind: 'error'; reason: ErrorReason; partial: bool }
export const swimEvents = eventSchema<SwimEvent>('swim.events', 1).roles({
  command: 'command',
  byte: 'data',
  reset: 'framing',
  entry: 'framing',
  calibration: 'control',
  error: 'error',
})
function validate(c: SwimStm8Config): string | undefined {
  if (c.data >= 64) return 'decode.config.channel'
  if (c.clockHz === 0 || c.clockHz > 100000000) return 'decode.config.clockHz'
  return undefined
}
class SwimStm8 extends LogicDecoder<SwimStm8Config, SwimEvent> {
  readonly watch = mask(this.config.data)
  clock: u32 = this.config.clockHz
  speed: Speed = this.config.speed
  active = this.config.startActive
  low: Tick | undefined = undefined
  partial = false
  packet: Tick | undefined = undefined
  direction: Direction = 'host-to-target'
  bits: u8 = 3
  seen: u8 = 0
  value: u8 = 0
  parity = false
  observedParity = false
  command: u8 | undefined = undefined
  step: u8 = 0
  count: u8 = 0
  address: u32 = 0
  offset: u8 = 0
  nextSpeed: Speed | undefined = undefined
  armed = false
  entryCount: u8 = 0
  entryStart: Tick | undefined = undefined
  entryRelease: Tick | undefined = undefined
  syncAfter: SyncAfter | undefined = undefined
  override hold(): Tick | undefined {
    let first = this.low
    const packet = this.packet
    const entry = this.entryStart
    if (packet !== undefined && (first === undefined || packet < first)) first = packet
    if (entry !== undefined && (first === undefined || entry < first)) first = entry
    return first
  }
  contextClear(): void {
    this.command = undefined
    this.step = 0
    this.count = 0
    this.address = 0
    this.offset = 0
    this.nextSpeed = undefined
  }
  async error(start: Tick, end: Tick, reason: ErrorReason, partial: bool): Promise<void> {
    await this.out.emit({ kind: 'error', reason, partial }, over(start, end, end))
  }
  async cut(tick: Tick): Promise<void> {
    const packet = this.packet
    const low = this.low
    const entry = this.entryStart
    if (packet !== undefined) await this.error(packet, tick, 'aborted', false)
    else if (low !== undefined) await this.error(low, tick, 'aborted', this.partial)
    else if (entry !== undefined) await this.error(entry, tick, 'aborted', false)
    this.packet = undefined
    this.low = undefined
    this.entryStart = undefined
    this.entryRelease = undefined
    this.entryCount = 0
    this.armed = false
    this.syncAfter = undefined
    this.active = false
    this.contextClear()
  }
  override async onGap(range: Range): Promise<void> {
    await this.cut(range.start)
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
  }
  width(ticks: u64, us: u64): bool {
    return (
      ticks >= mulDivCeil(this.rate, us * 3, 4000000) && ticks <= mulDiv(this.rate, us * 5, 4000000)
    )
  }
  field(): Field {
    if (this.step === 0) return 'count'
    if (this.step === 1) return 'address-extended'
    if (this.step === 2) return 'address-high'
    if (this.step === 3) return 'address-low'
    return 'data'
  }
  async complete(ack: bool, rise: Tick): Promise<void> {
    const first = this.packet
    if (first === undefined) return
    const parityOk = this.parity === this.observedParity
    const accepted = parityOk && ack
    if (this.bits === 3) {
      if (accepted) {
        this.contextClear()
        if (this.value === 1 || this.value === 2) this.command = this.value
      }
      await this.out.emit(
        { kind: 'command', value: this.value, parityOk, ack },
        over(first, rise + 1, rise),
      )
      this.packet = undefined
      return
    }
    const command = this.command
    const known =
      command !== undefined &&
      ((this.direction === 'host-to-target' && (this.step < 4 || command === 2)) ||
        (this.direction === 'target-to-host' && this.step >= 4 && command === 1))
    const field: Field = known ? this.field() : 'unclassified'
    const address: u32 | undefined =
      known && this.step >= 4 ? (this.address + this.offset) & 0xffffff : undefined
    await this.out.emit(
      { kind: 'byte', direction: this.direction, field, value: this.value, parityOk, ack, address },
      over(first, rise + 1, rise),
    )
    if (accepted && known) {
      let done = false
      if (this.step === 0) {
        this.count = this.value
        this.step = 1
        if (this.value === 0) {
          await this.error(first, rise + 1, 'invalid-count', false)
          done = true
        }
      } else if (this.step < 4) {
        this.address = ((this.address << 8) | (this.value as u32)) & 0xffffff
        this.step += 1
      } else {
        this.offset += 1
        done = this.offset >= this.count
        if (field === 'data' && address === 0x7f80 && command === 2)
          this.nextSpeed = (this.value & 16) !== 0 ? 'high' : 'low'
      }
      if (done) {
        const next = this.nextSpeed
        if (next !== undefined) this.speed = next
        this.contextClear()
      }
    }
    this.packet = undefined
  }
  async pulse(fall: Tick, rise: Tick, partial: bool): Promise<void> {
    const width: u64 = rise - fall
    if (this.armed) {
      const half: u64 = this.entryCount < 4 ? 500 : 250
      const highHalf: u64 = this.entryCount <= 4 ? 500 : 250
      const release = this.entryRelease
      let valid = this.width(width, half)
      if (this.entryCount > 0) {
        if (release !== undefined) {
          if (!this.width(fall - release, highHalf)) valid = false
        } else valid = false
      }
      if (valid && !partial) {
        if (this.entryCount === 0) this.entryStart = fall
        this.entryCount += 1
        this.entryRelease = rise
        if (this.entryCount === 8) {
          const first = this.entryStart
          this.packet = undefined
          this.contextClear()
          this.speed = 'low'
          this.active = true
          this.armed = false
          this.entryCount = 0
          this.entryStart = undefined
          this.entryRelease = undefined
          this.syncAfter = 'entry'
          if (first !== undefined)
            await this.out.emit({ kind: 'entry' }, over(first, rise + 1, rise))
        }
        return
      }
      this.armed = false
      this.entryCount = 0
      this.entryStart = undefined
      this.entryRelease = undefined
    }
    const after = this.syncAfter
    if (after !== undefined && this.config.calibrateSync && !partial && width > 0) {
      this.syncAfter = undefined
      const clock = mulDiv(this.rate, 128, width)
      if (clock > 0 && clock <= 100000000 && clock <= this.rate) {
        const packet = this.packet
        if (packet !== undefined) await this.error(packet, rise, 'reset', false)
        this.clock = clock as u32
        this.packet = undefined
        this.contextClear()
        this.speed = 'low'
        this.active = true
        await this.out.emit(
          { kind: 'calibration', clockHz: this.clock, lowTicks: width, after },
          over(fall, rise + 1, rise),
        )
        return
      }
    }
    if (width > mulDiv(this.rate, 64, this.clock as u64)) {
      const packet = this.packet
      if (packet !== undefined) await this.error(packet, rise, 'reset', false)
      this.packet = undefined
      this.contextClear()
      this.speed = 'low'
      this.active = true
      await this.out.emit({ kind: 'reset' }, over(fall, rise + 1, rise))
      this.armed = !partial && width >= mulDivCeil(this.rate, 16, 1000000)
      this.syncAfter = partial ? undefined : 'reset'
      return
    }
    if (partial) {
      await this.error(fall, rise, 'aborted', true)
      this.active = false
      this.contextClear()
      return
    }
    if (!this.active) return
    const threshold: u64 = this.speed === 'low' ? 8 : 4
    const bit = width <= mulDiv(this.rate, threshold, this.clock as u64)
    const packet = this.packet
    if (packet === undefined) {
      this.packet = fall
      this.direction = bit ? 'target-to-host' : 'host-to-target'
      const byte = bit || (this.command !== undefined && (this.step < 4 || this.command === 2))
      this.bits = byte ? 8 : 3
      this.seen = 0
      this.value = 0
      this.parity = false
      this.observedParity = false
      return
    }
    if (this.seen < this.bits) {
      this.value = (this.value << 1) | (bit ? 1 : 0)
      this.parity = this.parity !== bit
    } else if (this.seen === this.bits) this.observedParity = bit
    else {
      await this.complete(bit, rise)
      return
    }
    this.seen += 1
  }
  override async run(): Promise<void> {
    const tick = this.signal.position
    const levels = await this.signal.sample()
    if (!levels.high(this.config.data)) {
      this.low = tick
      this.partial = true
      this.active = false
    }
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      if (edge.levels.high(this.config.data)) {
        const low = this.low
        const partial = this.partial
        this.low = undefined
        this.partial = false
        if (low !== undefined) await this.pulse(low, edge.tick, partial)
      } else {
        this.low = edge.tick
        this.partial = false
      }
    }
  }
}
export const swimStm8 = defineLogicDecoder({
  name: 'swim-stm8',
  category: 'bus',
  config: {
    defaults: { data: 0, clockHz: 8000000, speed: 'low', startActive: true, calibrateSync: false },
    validate,
  },
  requires: (config) => ({ channels: atLeast(config.data + 1), rateHz: atLeast(config.clockHz) }),
  output: swimEvents,
  props: () => ({ bitOrder: 'msb-first' }),
  decoder: SwimStm8,
  locales: {
    'en-US': {
      name: 'STM8 SWIM (sync and recovery)',
      summary:
        'SWIM packets, command context, observed speed writes and optional synchronization calibration',
      fields: {
        data: 'SWIM',
        clockHz: 'Initial SWIM clock (Hz)',
        speed: 'Initial communication speed',
        startActive: 'Start at an active command boundary',
        calibrateSync: 'Trust the next low pulse as synchronization after entry/reset',
      },
      values: { speed: { low: 'Low speed', high: 'High speed' } },
      events: {
        command: 'Command',
        byte: 'Byte',
        reset: 'Reset or synchronization',
        entry: 'Entry sequence',
        calibration: 'Synchronization clock estimate',
        error: 'Error',
      },
      eventFields: {
        value: 'Value',
        parityOk: 'Parity valid',
        ack: 'ACK',
        direction: 'Direction',
        field: 'Command position',
        address: 'Observed address',
        clockHz: 'Estimated clock (Hz)',
        lowTicks: 'Synchronization low ticks',
        after: 'Following sequence',
        reason: 'Reason',
        partial: 'Partial initial low pulse',
      },
    },
    'zh-CN': {
      name: 'STM8 SWIM（同步与恢复）',
      summary: 'SWIM 数据包、命令上下文、观察到的速度写入与可选同步校准',
      fields: {
        data: 'SWIM',
        clockHz: '初始 SWIM 时钟（Hz）',
        speed: '初始通信速度',
        startActive: '从活动命令边界开始',
        calibrateSync: '将进入或复位后的下个低脉冲视为同步',
      },
      values: { speed: { low: '低速', high: '高速' } },
      events: {
        command: '命令',
        byte: '字节',
        reset: '复位或同步',
        entry: '进入序列',
        calibration: '同步时钟估计',
        error: '错误',
      },
      eventFields: {
        value: '值',
        parityOk: '奇偶校验有效',
        ack: 'ACK',
        direction: '方向',
        field: '命令位置',
        address: '观察到的地址',
        clockHz: '估计时钟（Hz）',
        lowTicks: '同步低电平采样数',
        after: '前置序列',
        reason: '原因',
        partial: '起始低脉冲不完整',
      },
    },
  },
})
