// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Explicit integer casts retain SFDP field widths in the IR. */

/** Phase-aware SDR flash from Winbond W25Q128JV rev F §§8.2.6–8.2.11,
 * figures18/20/22/24: https://www.winbond.com/resource-files/w25q128jv%20revf%2003272018%20plus.pdf .
 * Explicit SPI/QPI command width. BB/EB switch address/mode lanes; 3B/6B switch data.
 * QPI framing: original Winbond W25Q128FV rev H §§8.2.7/11/34,
 * https://www.pjrc.com/teensy/W25Q128FV.pdf . Observed status readback gates QPI entry;
 * command transitions are candidates, not verified device effects. No automatic
 * continuous-read state, voltage or timing inference. DTR 0D/BD/ED phases
 * use Winbond W25Q128JV-DTR rev B instruction tables 4/5 and figures17/23/27:
 * https://cdn.sparkfun.com/assets/5/b/2/a/6/W25Q128JV_Datasheet.pdf .
 * SFDP headers/basic density: Microchip SST26VF064B appendix table8-1:
 * https://onlinedocs.microchip.com/oxy/GUID-EF5727DA-42CC-4107-8984-4D8EAF42FF5D-en-US-1/GUID-0394DD94-818C-41EF-BD6C-0AB8FB863C2A.html . */
import type { Channel, Levels, Range, Tick, Provenance, bool, u8, u16, u32, u64 } from 'uiharu'
import {
  LogicDecoder,
  at,
  atLeast,
  defineLogicDecoder,
  eventSchema,
  fromBool,
  mask,
  max,
  min,
  one,
  over,
} from 'uiharu'

export type StatusField =
  | 'busy'
  | 'write-enable'
  | 'block-protect'
  | 'top-bottom'
  | 'sector-protect'
  | 'status-protect'
  | 'quad-enable'
  | 'suspended'
  | 'complement-protect'
  | 'security-lock'
  | 'write-protect-select'
  | 'driver-strength'
  | 'hold-reset'
export type FlashEnd = 'complete' | 'incomplete' | 'aborted' | 'unsupported'
export type FlashEvent =
  | { kind: 'begin'; tx: u64; partial: bool }
  | {
      kind: 'mode-request'
      tx: u64
      from: CommandMode
      to: CommandMode
      quadEnable: bool | undefined
      followed: bool
    }
  | { kind: 'address-mode'; tx: u64; bytes: u8; reset: bool }
  | { kind: 'command'; tx: u64; opcode: u8; read: bool; lanes: u8 }
  | { kind: 'address'; tx: u64; value: u32; bits: u8; lanes: u8 }
  | { kind: 'mode'; tx: u64; value: u8 }
  | { kind: 'dummy'; tx: u64; clocks: u8 }
  | { kind: 'data'; tx: u64; offset: u32; value: u8; read: bool; lanes: u8 }
  | {
      kind: 'sfdp-header'
      tx: u64
      signature: u32
      valid: bool
      minor: u8
      major: u8
      parameterHeaders: u16
    }
  | {
      kind: 'sfdp-parameter'
      tx: u64
      index: u16
      id: u16
      minor: u8
      major: u8
      dwords: u8
      pointer: u32
    }
  | { kind: 'density'; tx: u64; raw: u32; bits: u64 | undefined }
  | { kind: 'erase'; tx: u64; address: u32 | undefined; bytes: u32 | undefined }
  | { kind: 'status'; tx: u64; register: u8; value: u8; written: bool }
  | { kind: 'status-bit'; tx: u64; field: StatusField; value: u8 }
  | { kind: 'unknown-command'; tx: u64; opcode: u8 }
  | { kind: 'end'; tx: u64; reason: FlashEnd; droppedBits: u8 }
export const flashEvents = eventSchema<FlashEvent>('qspi-flash.events', 1).roles({
  begin: 'framing',
  'mode-request': 'control',
  'address-mode': 'control',
  command: 'command',
  address: 'address',
  mode: 'data',
  dummy: 'framing',
  data: 'data',
  'sfdp-header': 'data',
  'sfdp-parameter': 'data',
  density: 'data',
  erase: 'command',
  status: 'data',
  'status-bit': 'data',
  'unknown-command': 'framing',
  end: 'framing',
})
export type CommandMode = 'spi' | 'qpi'
export interface QspiFlashConfig {
  commandMode: CommandMode
  qpiDummyClocks: u8
  dtrQuadDummyClocks: u8
  clk: Channel
  cs: Channel
  io0: Channel
  io1: Channel
  io2: Channel
  io3: Channel
  addressBytes: u8
  quadDummyClocks: u8
  mode: u8
}
type Phase = 'opcode' | 'address' | 'mode' | 'dummy' | 'data' | 'done' | 'unknown' | 'partial'
function validate(config: QspiFlashConfig): string | undefined {
  if (config.addressBytes !== 3 && config.addressBytes !== 4)
    return 'qspi-flash.config.addressBytes'
  if (config.dtrQuadDummyClocks > 32) return 'qspi-flash.config.dtrQuadDummyClocks'
  if (config.qpiDummyClocks < 2 || config.qpiDummyClocks > 8 || config.qpiDummyClocks % 2 !== 0)
    return 'qspi-flash.config.qpiDummyClocks'
  if ((config.mode !== 0 && config.mode !== 3) || config.quadDummyClocks > 64)
    return 'qspi-flash.config.mode'
  if (
    max(
      max(config.clk, config.cs),
      max(max(config.io0, config.io1), max(config.io2, config.io3)),
    ) >= 64
  )
    return 'qspi-flash.config.channel'
  if (
    config.clk === config.cs ||
    config.clk === config.io0 ||
    config.clk === config.io1 ||
    config.clk === config.io2 ||
    config.clk === config.io3 ||
    config.cs === config.io0 ||
    config.cs === config.io1 ||
    config.cs === config.io2 ||
    config.cs === config.io3 ||
    config.io0 === config.io1 ||
    config.io0 === config.io2 ||
    config.io0 === config.io3 ||
    config.io1 === config.io2 ||
    config.io1 === config.io3 ||
    config.io2 === config.io3
  )
    return 'qspi-flash.config.duplicateChannel'
  return undefined
}
function baseOpcode(opcode: u8): u8 {
  if (opcode === 0x13) return 0x03
  if (opcode === 0x0c) return 0x0b
  if (opcode === 0x3c) return 0x3b
  if (opcode === 0x6c) return 0x6b
  if (opcode === 0xbc) return 0xbb
  if (opcode === 0xec) return 0xeb
  if (opcode === 0x12) return 0x02
  if (opcode === 0x34) return 0x32
  return opcode
}
class QspiFlash extends LogicDecoder<QspiFlashConfig, FlashEvent> {
  readonly watch = mask(this.config.clk, this.config.cs)
  commandMode: CommandMode = this.config.commandMode
  modeKnown = true
  currentAddressBytes: u8 = this.config.addressBytes
  quadEnable: bool | undefined = undefined
  resetEnabled = false
  resetWasEnabled = false
  trailing = false
  statusQe: bool | undefined = undefined
  qeWritten = false

  sfHeld: Tick | undefined = undefined
  sfSignature: u32 = 0
  sfPart: u32 = 0
  sfMinor: u8 = 0
  sfMajor: u8 = 0
  sfCount: u16 = 0
  sfKnown = false
  sfId: u16 = 0
  sfDwords: u8 = 0
  sfPointer: u32 = 0
  basicPointer: u32 | undefined = undefined
  opcode: u8 = 0
  addressBits: u8 = 24
  address: u32 = 0
  eraseBytes: u32 | undefined = undefined
  statusRegister: u8 = 0
  tx: u64 = 0
  open = false
  phase: Phase = 'done'
  lanes: u8 = 1
  dtr = false
  skipTrailing = false
  dataLanes: u8 = 1
  read = false
  addressLanes: u8 = 1
  modeNeeded = false
  dummyClocks: u8 = 0
  left: u8 = 0
  bits: u8 = 0
  value: u32 = 0
  first: Tick = 0
  offset: u32 = 0
  override hold(): Tick | undefined {
    const first: Tick | undefined = this.bits > 0 ? this.first : undefined
    if (this.sfHeld !== undefined)
      return first === undefined ? this.sfHeld : min(first, this.sfHeld)
    return first
  }
  override async onGap(range: Range): Promise<void> {
    await this.end(range.start, 'aborted')
    this.sfKnown = false
    this.basicPointer = undefined
    this.commandMode = this.config.commandMode
    this.modeKnown = false
    this.currentAddressBytes = this.config.addressBytes
    this.quadEnable = undefined
    this.resetEnabled = false
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.end(tick, 'aborted')
  }
  async end(tick: Tick, reason: FlashEnd): Promise<void> {
    if (!this.open) return
    this.open = false
    if (reason === 'complete') await this.completed(tick)
    await this.out.emit(
      { kind: 'end', tx: this.tx, reason, droppedBits: this.bits },
      reason === 'aborted' ? at(tick) : one(tick),
    )
    this.bits = 0
    this.sfHeld = undefined
  }
  async completed(tick: Tick): Promise<void> {
    if (this.trailing) return
    if (this.statusQe !== undefined) this.quadEnable = this.statusQe
    if (this.qeWritten && this.commandMode !== 'qpi') this.quadEnable = undefined
    if (this.opcode === 0x38 || (this.opcode === 0xff && this.commandMode === 'qpi')) {
      const from = this.commandMode
      const to: CommandMode = this.opcode === 0x38 ? 'qpi' : 'spi'
      const followed = to === 'spi' || this.quadEnable === true
      await this.out.emit(
        { kind: 'mode-request', tx: this.tx, from, to, quadEnable: this.quadEnable, followed },
        one(tick),
      )
      if (followed) this.commandMode = to
      else if (this.quadEnable === undefined) this.modeKnown = false
    }
    if (this.opcode === 0xb7 || this.opcode === 0xe9) {
      this.currentAddressBytes = this.opcode === 0xb7 ? 4 : 3
      await this.out.emit(
        { kind: 'address-mode', tx: this.tx, bytes: this.currentAddressBytes, reset: false },
        one(tick),
      )
    }
    if (this.opcode === 0x66) this.resetEnabled = true
    if (this.opcode === 0x99 && this.resetWasEnabled) {
      const from = this.commandMode
      this.commandMode = 'spi'
      this.currentAddressBytes = this.config.addressBytes
      this.quadEnable = undefined
      await this.out.emit(
        {
          kind: 'mode-request',
          tx: this.tx,
          from,
          to: 'spi',
          quadEnable: undefined,
          followed: true,
        },
        one(tick),
      )
      await this.out.emit(
        { kind: 'address-mode', tx: this.tx, bytes: this.currentAddressBytes, reset: true },
        one(tick),
      )
    }
  }
  async begin(tick: Tick, partial: bool): Promise<void> {
    this.sfHeld = undefined
    this.resetWasEnabled = this.resetEnabled
    this.resetEnabled = false
    this.trailing = false
    this.statusQe = undefined
    this.qeWritten = false
    this.dtr = false
    this.skipTrailing = false
    this.tx += 1
    this.open = true
    const uncertain = partial || !this.modeKnown
    this.phase = uncertain ? 'partial' : 'opcode'
    this.lanes = this.commandMode === 'qpi' ? 4 : 1
    this.bits = 0
    this.value = 0
    this.offset = 0
    await this.out.emit({ kind: 'begin', tx: this.tx, partial: uncertain }, one(tick))
  }
  layout(opcode: u8): bool {
    this.opcode = opcode
    const operation = this.commandMode === 'qpi' && opcode === 0x0c ? 0x0b : baseOpcode(opcode)
    const dedicatedFour = this.commandMode === 'spi' && operation !== opcode
    this.statusRegister = 0
    this.eraseBytes = undefined
    this.addressBits = dedicatedFour ? 32 : this.currentAddressBytes * 8
    this.address = 0
    this.addressLanes = 1
    this.dataLanes = 1
    this.read = false
    this.modeNeeded = false
    this.dummyClocks = 0
    this.left = this.addressBits
    if (operation === 0x5a && this.commandMode === 'spi') {
      this.read = true
      this.addressBits = 24
      this.dummyClocks = 8
      this.lanes = 1
      this.phase = 'address'
      return true
    }
    if (
      operation === 0x20 ||
      operation === 0x21 ||
      operation === 0x52 ||
      operation === 0xd8 ||
      operation === 0xdc
    ) {
      this.eraseBytes =
        operation === 0x20 || operation === 0x21 ? 4096 : operation === 0x52 ? 32768 : 65536
      this.addressBits =
        operation === 0x21 || operation === 0xdc ? 32 : this.currentAddressBytes * 8
      this.addressLanes = this.commandMode === 'qpi' ? 4 : 1
      this.lanes = this.addressLanes
      this.phase = 'address'
      return true
    }
    if (
      operation === 0x44 ||
      operation === 0x36 ||
      operation === 0x39 ||
      operation === 0x42 ||
      operation === 0x48 ||
      operation === 0x3d ||
      operation === 0x90
    ) {
      this.addressLanes = this.commandMode === 'qpi' ? 4 : 1
      this.dataLanes = this.addressLanes
      this.lanes = this.addressLanes
      this.read = operation === 0x48 || operation === 0x3d || operation === 0x90
      if (operation === 0x90) this.addressBits = 24
      this.left = this.addressBits
      if (operation === 0x44) this.eraseBytes = 256
      if (operation === 0x48)
        this.dummyClocks = this.commandMode === 'qpi' ? this.config.qpiDummyClocks : 8
      this.phase = 'address'
      return true
    }
    if (this.commandMode === 'spi' && (operation === 0x92 || operation === 0x94)) {
      this.read = true
      this.addressBits = 24
      this.addressLanes = operation === 0x92 ? 2 : 4
      this.dataLanes = this.addressLanes
      this.lanes = this.addressLanes
      this.modeNeeded = true
      this.dummyClocks = operation === 0x94 ? 4 : 0
      this.phase = 'address'
      return true
    }
    if (this.commandMode === 'spi' && operation === 0x77) {
      this.read = false
      this.dataLanes = 4
      this.lanes = 1
      this.dummyClocks = 6
      this.left = 6
      this.phase = 'dummy'
      return true
    }
    if (operation === 0xab) {
      this.read = true
      this.dataLanes = this.commandMode === 'qpi' ? 4 : 1
      this.lanes = 1
      this.phase = 'dummy'
      this.dummyClocks = this.commandMode === 'qpi' ? 6 : 24
      this.left = this.dummyClocks
      return true
    }
    if (operation === 0xc8 || operation === 0xc5 || operation === 0x4b) {
      this.read = operation !== 0xc5
      this.dataLanes = this.commandMode === 'qpi' ? 4 : 1
      this.lanes = this.dataLanes
      this.phase = operation === 0x4b ? 'dummy' : 'data'
      this.dummyClocks = operation === 0x4b ? 32 : 0
      this.left = this.dummyClocks
      return true
    }
    if (
      operation === 0x50 ||
      operation === 0x75 ||
      operation === 0x7a ||
      operation === 0xb9 ||
      operation === 0x7e ||
      operation === 0x98 ||
      operation === 0xb7 ||
      operation === 0xe9 ||
      (operation === 0x38 && this.commandMode === 'spi')
    ) {
      this.phase = 'done'
      return true
    }
    if (operation === 0x60 || operation === 0xc7) {
      this.phase = 'done'
      return true
    }
    if (
      operation === 0x05 ||
      operation === 0x35 ||
      operation === 0x15 ||
      operation === 0x01 ||
      operation === 0x31 ||
      operation === 0x11
    ) {
      this.statusRegister =
        operation === 0x05 || operation === 0x01
          ? 1
          : operation === 0x35 || operation === 0x31
            ? 2
            : 3
      this.read = operation === 0x05 || operation === 0x35 || operation === 0x15
      this.left = 0
      this.dataLanes = this.commandMode === 'qpi' ? 4 : 1
      this.lanes = this.dataLanes
      this.phase = 'data'
      return true
    }
    if (operation === 0x0d || operation === 0xbd || operation === 0xed) {
      if (this.commandMode === 'qpi' && operation === 0xbd) return false
      this.dtr = true
      this.skipTrailing = true
      this.read = true
      this.addressLanes =
        operation === 0xbd ? 2 : operation === 0xed || this.commandMode === 'qpi' ? 4 : 1
      this.dataLanes = this.addressLanes
      this.modeNeeded = opcode !== 0x0d
      this.dummyClocks =
        operation === 0xed
          ? this.config.dtrQuadDummyClocks
          : operation === 0xbd
            ? 4
            : this.commandMode === 'qpi'
              ? 8
              : 6
      this.phase = 'address'
      this.lanes = this.addressLanes
      return true
    }
    if (this.commandMode === 'qpi') {
      this.addressLanes = 4
      this.dataLanes = 4
      if (operation === 0x0b || operation === 0xeb || operation === 0x0c) {
        this.read = true
        this.dummyClocks = this.config.qpiDummyClocks
        if (operation === 0xeb) {
          this.modeNeeded = true
          this.dummyClocks -= 2
        }
      } else if (operation === 0x02) {
      } else if (
        operation === 0x05 ||
        operation === 0x35 ||
        operation === 0x15 ||
        operation === 0x9f
      ) {
        this.read = true
        this.left = 0
      } else if (operation === 0xc0) {
        this.left = 0
      } else if (
        operation === 0x06 ||
        operation === 0x04 ||
        operation === 0x66 ||
        operation === 0x99 ||
        operation === 0xff
      ) {
        this.left = 0
        this.phase = 'done'
        return true
      } else return false
      this.phase = this.left > 0 ? 'address' : 'data'
      this.lanes = this.left > 0 ? this.addressLanes : this.dataLanes
      return true
    }
    switch (operation) {
      case 0x03:
        this.read = true
        break
      case 0x0b:
        this.read = true
        this.dummyClocks = 8
        break
      case 0x3b:
        this.read = true
        this.dummyClocks = 8
        this.dataLanes = 2
        break
      case 0x6b:
        this.read = true
        this.dummyClocks = 8
        this.dataLanes = 4
        break
      case 0xbb:
        this.read = true
        this.addressLanes = 2
        this.dataLanes = 2
        this.modeNeeded = true
        break
      case 0xeb:
        this.read = true
        this.addressLanes = 4
        this.dataLanes = 4
        this.modeNeeded = true
        this.dummyClocks = this.config.quadDummyClocks
        break
      case 0x02:
        break
      case 0x32:
        this.dataLanes = 4
        break
      case 0x05:
      case 0x35:
      case 0x15:
      case 0x9f:
        this.read = true
        this.left = 0
        break
      case 0x06:
      case 0x04:
      case 0x66:
      case 0x99:
      case 0xb7:
      case 0xe9:
        this.left = 0
        this.phase = 'done'
        return true
      default:
        return false
    }
    this.phase = this.left > 0 ? 'address' : 'data'
    this.lanes = this.left > 0 ? this.addressLanes : this.dataLanes
    return true
  }
  afterAddress(): void {
    if (this.modeNeeded) {
      this.phase = 'mode'
      this.left = 8
      this.lanes = this.addressLanes
    } else if (this.dummyClocks > 0) {
      this.phase = 'dummy'
      this.left = this.dummyClocks * (this.dtr ? 2 : 1)
      this.lanes = 1
    } else {
      this.phase = 'data'
      this.lanes = this.dataLanes
    }
  }
  group(levels: Levels): u8 {
    if (this.lanes === 1)
      return fromBool(
        levels.high(this.phase === 'data' && this.read ? this.config.io1 : this.config.io0),
      )
    let value: u8 =
      fromBool(levels.high(this.config.io0)) | (fromBool(levels.high(this.config.io1)) << 1)
    if (this.lanes === 4)
      value |=
        (fromBool(levels.high(this.config.io2)) << 2) |
        (fromBool(levels.high(this.config.io3)) << 3)
    return value
  }
  async sfdp(value: u8, tick: Tick): Promise<void> {
    const address = this.address + this.offset
    if (address < 8) {
      if (address === 0) {
        this.sfHeld = this.first
        this.sfSignature = 0
        this.sfKnown = false
        this.sfCount = 0
      }
      if (this.sfHeld === undefined) return
      if (address < 4) this.sfSignature |= (value as u32) << (address * 8)
      else if (address === 4) this.sfMinor = value
      else if (address === 5) this.sfMajor = value
      else if (address === 6) this.sfCount = (value as u16) + 1
      else {
        this.sfKnown = this.sfSignature === 0x50444653
        await this.out.emit(
          {
            kind: 'sfdp-header',
            tx: this.tx,
            signature: this.sfSignature,
            valid: this.sfKnown,
            minor: this.sfMinor,
            major: this.sfMajor,
            parameterHeaders: this.sfCount,
          },
          over(this.sfHeld, tick + 1, tick),
        )
        this.sfHeld = undefined
      }
      return
    }
    if (this.sfKnown && address < 8 + this.sfCount * 8) {
      const position: u8 = ((address - 8) % 8) as u8
      if (position === 0) {
        this.sfHeld = this.first
        this.sfId = value
        this.sfPointer = 0
      }
      if (this.sfHeld === undefined) return
      if (position === 1) this.sfMinor = value
      else if (position === 2) this.sfMajor = value
      else if (position === 3) this.sfDwords = value
      else if (position >= 4 && position <= 6)
        this.sfPointer |= (value as u32) << ((position - 4) * 8)
      else if (position === 7) {
        this.sfId |= (value as u16) << 8
        await this.out.emit(
          {
            kind: 'sfdp-parameter',
            tx: this.tx,
            index: ((address - 8) / 8) as u16,
            id: this.sfId,
            minor: this.sfMinor,
            major: this.sfMajor,
            dwords: this.sfDwords,
            pointer: this.sfPointer,
          },
          over(this.sfHeld, tick + 1, tick),
        )
        this.sfHeld = undefined
        if (this.sfId === 0xff00 && this.sfDwords >= 2 && (this.sfPointer & 3) === 0)
          this.basicPointer = this.sfPointer
      }
      return
    }
    const pointer = this.basicPointer
    if (pointer === undefined) return
    if (address < pointer + 4 || address >= pointer + 8) return
    const position: u8 = (address - pointer - 4) as u8
    if (position === 0) {
      this.sfPart = 0
      this.sfHeld = this.first
    }
    if (this.sfHeld === undefined) return
    this.sfPart |= (value as u32) << (position * 8)
    if (position === 3) {
      let bits: u64 | undefined = undefined
      if ((this.sfPart & 0x80000000) === 0) bits = (this.sfPart as u64) + 1
      else if ((this.sfPart & 0x7fffffff) < 64) bits = (1 as u64) << (this.sfPart & 0x7fffffff)
      await this.out.emit(
        { kind: 'density', tx: this.tx, raw: this.sfPart, bits },
        over(this.sfHeld, tick + 1, tick),
      )
      this.sfHeld = undefined
    }
  }
  async statusBits(value: u8, where: Provenance): Promise<void> {
    const register: u8 =
      this.statusRegister === 1 && this.offset > 0 && !this.read ? 2 : this.statusRegister
    await this.out.emit(
      { kind: 'status', tx: this.tx, register, value, written: !this.read },
      where,
    )
    if (register === 1) {
      await this.statusBit('busy', value & 1, where)
      await this.statusBit('write-enable', (value >> 1) & 1, where)
      await this.statusBit('block-protect', (value >> 2) & 7, where)
      await this.statusBit('top-bottom', (value >> 5) & 1, where)
      await this.statusBit('sector-protect', (value >> 6) & 1, where)
      await this.statusBit('status-protect', value >> 7, where)
    } else if (register === 2) {
      if (this.read) this.statusQe = (value & 2) !== 0
      else this.qeWritten = true
      await this.statusBit('quad-enable', (value >> 1) & 1, where)
      await this.statusBit('suspended', value >> 7, where)
      await this.statusBit('complement-protect', (value >> 6) & 1, where)
      await this.statusBit('security-lock', (value >> 3) & 7, where)
      await this.statusBit('status-protect', value & 1, where)
    } else {
      await this.statusBit('write-protect-select', (value >> 2) & 1, where)
      await this.statusBit('driver-strength', (value >> 5) & 3, where)
      await this.statusBit('hold-reset', value >> 7, where)
    }
  }
  async statusBit(field: StatusField, value: u8, where: Provenance): Promise<void> {
    await this.out.emit({ kind: 'status-bit', tx: this.tx, field, value }, where)
  }
  async shift(tick: Tick, levels: Levels): Promise<void> {
    if (this.phase === 'done') {
      this.trailing = true
      return
    }
    if (this.phase === 'partial' || this.phase === 'unknown') return
    if (this.phase === 'dummy') {
      if (this.bits === 0) this.first = tick
      this.bits += 1
      this.left -= 1
      if (this.left === 0) {
        await this.out.emit(
          { kind: 'dummy', tx: this.tx, clocks: this.dummyClocks },
          over(this.first, tick + 1, tick),
        )
        this.bits = 0
        this.value = 0
        this.phase = 'data'
        this.lanes = this.dataLanes
      }
      return
    }
    if (this.bits === 0) this.first = tick
    this.value = (this.value << this.lanes) | this.group(levels)
    this.bits += this.lanes
    const width: u8 = this.phase === 'address' ? this.addressBits : 8
    if (this.bits < width) return
    const where = over(this.first, tick + 1, tick)
    const value = this.value
    const bits = this.bits
    this.bits = 0
    this.value = 0
    switch (this.phase) {
      case 'opcode':
        if (this.layout(value as u8))
          await this.out.emit(
            {
              kind: 'command',
              tx: this.tx,
              opcode: value as u8,
              read: this.read,
              lanes: this.dataLanes,
            },
            where,
          )
        else {
          this.phase = 'unknown'
          await this.out.emit({ kind: 'unknown-command', tx: this.tx, opcode: value as u8 }, where)
        }
        if (this.opcode === 0x60 || this.opcode === 0xc7)
          await this.out.emit(
            { kind: 'erase', tx: this.tx, address: undefined, bytes: undefined },
            where,
          )
        break
      case 'address':
        this.address = value
        await this.out.emit({ kind: 'address', tx: this.tx, value, bits, lanes: this.lanes }, where)
        const bytes = this.eraseBytes
        if (bytes !== undefined) {
          await this.out.emit({ kind: 'erase', tx: this.tx, address: value, bytes }, where)
          this.phase = 'done'
        } else if (this.opcode === 0x36 || this.opcode === 0x39) this.phase = 'done'
        else this.afterAddress()
        break
      case 'mode':
        await this.out.emit({ kind: 'mode', tx: this.tx, value: value as u8 }, where)
        this.modeNeeded = false
        this.afterAddress()
        break
      case 'data':
        if (this.opcode === 0x5a) await this.sfdp(value as u8, tick)
        if (this.statusRegister !== 0) await this.statusBits(value as u8, where)
        await this.out.emit(
          {
            kind: 'data',
            tx: this.tx,
            offset: this.offset,
            value: value as u8,
            read: this.read,
            lanes: this.lanes,
          },
          where,
        )
        this.offset += 1
        break
      default:
        break
    }
  }
  override async run(): Promise<void> {
    const initial = await this.signal.sample()
    if (!initial.high(this.config.cs)) await this.begin(this.signal.position, true)
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      if (edge.changed.has(this.config.cs)) {
        if (!edge.levels.high(this.config.cs)) await this.begin(edge.tick, false)
        else {
          let reason: FlashEnd = 'complete'
          if (this.phase === 'unknown') reason = 'unsupported'
          else if (
            this.phase === 'partial' ||
            this.phase === 'opcode' ||
            this.phase === 'address' ||
            this.phase === 'mode' ||
            this.phase === 'dummy' ||
            this.bits > 0 ||
            (this.phase === 'data' && !this.read && this.offset === 0)
          )
            reason = 'incomplete'
          if (this.trailing) reason = 'unsupported'
          if (this.opcode === 0xab && this.phase === 'dummy' && this.bits === 0) reason = 'complete'
          await this.end(edge.tick, reason)
        }
        continue
      }
      if (this.open && edge.changed.has(this.config.clk)) {
        const sample =
          edge.levels.high(this.config.clk) ===
          ((this.config.mode & 2) !== 0
            ? (this.config.mode & 1) !== 0
            : (this.config.mode & 1) === 0)
        if (this.dtr && this.skipTrailing) {
          this.skipTrailing = false
          continue
        }
        if (sample || this.dtr) await this.shift(edge.tick, edge.levels)
      }
    }
  }
}
export const qspiFlash = defineLogicDecoder({
  name: 'qspi-flash',
  category: 'device',
  config: {
    defaults: {
      clk: 1,
      cs: 0,
      io0: 2,
      io1: 3,
      io2: 4,
      io3: 5,
      addressBytes: 3,
      quadDummyClocks: 4,
      mode: 0,
      commandMode: 'spi',
      qpiDummyClocks: 2,
      dtrQuadDummyClocks: 7,
    },
    validate,
  },
  requires: (config) => ({
    channels: atLeast(
      max(
        max(config.clk, config.cs),
        max(max(config.io0, config.io1), max(config.io2, config.io3)),
      ) + 1,
    ),
  }),
  output: flashEvents,
  props: () => ({ phaseAware: true }),

  locales: {
    'en-US': {
      name: 'SPI / QSPI NOR flash',
      summary: 'NOR SDR/QPI/DTR phases, erase/status fields and SFDP discovery',
      fields: {
        commandMode: 'Command width',
        qpiDummyClocks: 'QPI dummy clocks',
        dtrQuadDummyClocks: 'DTR quad dummy clocks',
        clk: 'Clock channel',
        cs: 'Chip-select channel',
        io0: 'IO0 channel',
        io1: 'IO1 channel',
        io2: 'IO2 channel',
        io3: 'IO3 channel',
        addressBytes: 'Address Bytes',
        quadDummyClocks: 'Quad Dummy Clocks',
        mode: 'Mode',
      },
      events: {
        begin: 'Begin',
        'mode-request': 'Command mode request',
        'address-mode': 'Address mode request',
        command: 'Command',
        address: 'Address',
        mode: 'Mode',
        dummy: 'Dummy',
        data: 'Data',
        'sfdp-header': 'Sfdp header',
        'sfdp-parameter': 'Sfdp parameter',
        density: 'Density',
        erase: 'Erase',
        status: 'Status',
        'status-bit': 'Status bit',
        'unknown-command': 'Unknown command',
        end: 'End',
      },
      eventFields: {
        tx: 'Transaction',
        partial: 'Partial',
        opcode: 'Opcode',
        from: 'Previous command mode',
        to: 'Requested command mode',
        quadEnable: 'Observed quad enable',
        followed: 'Parser followed request',
        reset: 'Reset sequence',
        read: 'Read',
        lanes: 'Lanes',
        value: 'Value',
        bits: 'Bits',
        clocks: 'Clocks',
        offset: 'Offset',
        signature: 'Signature',
        valid: 'Valid',
        minor: 'Minor',
        major: 'Major',
        parameterHeaders: 'Parameter Headers',
        index: 'Index',
        id: 'Id',
        dwords: 'Dwords',
        pointer: 'Pointer',
        raw: 'Raw',
        address: 'Address',
        bytes: 'Bytes',
        register: 'Register',
        written: 'Written',
        field: 'Field',
        reason: 'Reason',
        droppedBits: 'Dropped Bits',
      },
      values: { commandMode: { spi: 'SPI', qpi: 'QPI' } },
    },
    'zh-CN': {
      name: 'SPI / QSPI NOR 闪存',
      summary: '解析NOR SDR、QPI、DTR阶段、擦除和状态字段及SFDP发现信息',
      fields: {
        commandMode: '命令宽度',
        qpiDummyClocks: 'QPI空时钟数',
        dtrQuadDummyClocks: 'DTR四线空时钟数',
        clk: '时钟通道',
        cs: '片选通道',
        io0: 'IO0通道',
        io1: 'IO1通道',
        io2: 'IO2通道',
        io3: 'IO3通道',
        addressBytes: '地址字节数',
        quadDummyClocks: '四线空时钟数',
        mode: '模式',
      },
      events: {
        begin: '开始',
        'mode-request': '命令模式请求',
        'address-mode': '地址模式请求',
        command: '命令',
        address: '地址',
        mode: '模式',
        dummy: '空位',
        data: '数据通道',
        'sfdp-header': 'SFDP头',
        'sfdp-parameter': 'SFDP参数头',
        density: '存储容量',
        erase: '擦除请求',
        status: '状态',
        'status-bit': '状态位',
        'unknown-command': '未知命令',
        end: '结束',
      },
      eventFields: {
        tx: '事务号',
        partial: '不完整',
        opcode: '操作码',
        from: '原命令模式',
        to: '请求命令模式',
        quadEnable: '观测四线使能',
        followed: '解码器跟随请求',
        reset: '复位序列',
        read: '读取',
        lanes: '数据线数',
        value: '数值',
        bits: '位数',
        clocks: '时钟数',
        offset: '偏移',
        signature: '签名',
        valid: '有效',
        minor: '次版本',
        major: '主版本',
        parameterHeaders: '参数头数量',
        index: '序号',
        id: '标识符',
        dwords: '双字数量',
        pointer: '表地址',
        raw: '原始值',
        address: '地址',
        bytes: '字节数',
        register: '寄存器',
        written: '已写入',
        field: '字段',
        reason: '结束原因',
        droppedBits: '丢弃位数',
      },
      values: { commandMode: { spi: 'SPI', qpi: 'QPI' } },
    },
  },
  decoder: QspiFlash,
})
