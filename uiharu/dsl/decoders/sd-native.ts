// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- DSL integer widths and empty configuration records carry IR meaning. */
/** Native SD SDR command, CID/CSD and read/write data transport from the SD Association
 * Physical Layer Simplified Specification v2.00 §§3.6,4.3–4.5,4.9,5:
 * https://chlazza.nfshost.com/docs/Simplified_Physical_Layer_Spec.pdf .
 * Four data lanes require explicit wiring. Auto width follows CRC-valid, accepted CMD55/ACMD6 exchanges. Command association is
 * observed host intent, not a claim the card accepted a write. No DDR, UHS-II, SDIO CMD53,
 * MMC/eMMC, CSD-v3 or tuning-pattern interpretation. State holds one byte and four CRCs. */
import type { Channel, Levels, Range, Tick, bool, u8, u16, u32, u64 } from 'uiharu'
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
export type End = 'complete' | 'invalid-end' | 'aborted'
export type StatusFormat = 'r1' | 'r1b' | 'r6'
export type StatusField =
  | 'out-of-range'
  | 'address-error'
  | 'block-length-error'
  | 'erase-sequence-error'
  | 'erase-parameter'
  | 'write-protect-violation'
  | 'card-locked'
  | 'lock-unlock-failed'
  | 'command-crc-error'
  | 'illegal-command'
  | 'ecc-failed'
  | 'controller-error'
  | 'general-error'
  | 'csd-overwrite'
  | 'write-protect-erase-skip'
  | 'ecc-disabled'
  | 'erase-reset'
  | 'current-state'
  | 'ready-for-data'
  | 'fx-event'
  | 'application-command'
  | 'authentication-sequence-error'
export type Voltage = 'not-defined' | 'v27-36' | 'reserved-low-voltage' | 'reserved'
export type Event =
  | {
      kind: 'card-status'
      token: u64
      format: StatusFormat
      value: u32
      relativeAddress: u16 | undefined
      currentState: u8
      readyForData: bool
      appCommand: bool
      crcOk: bool
      endOk: bool
    }
  | { kind: 'status-field'; token: u64; field: StatusField; value: u8 }
  | { kind: 'relative-address'; token: u64; address: u16; status: u16; crcOk: bool; endOk: bool }
  | {
      kind: 'interface-condition'
      token: u64
      voltage: u8
      voltageRange: Voltage
      pattern: u8
      expectedPattern: u8
      patternOk: bool
      voltageOk: bool
      reserved: u32
      crcOk: bool
      endOk: bool
    }
  | {
      kind: 'ocr'
      token: u64
      value: u32
      ready: bool
      highCapacity: bool
      voltageWindow: u32
      endOk: bool
    }
  | {
      kind: 'scr'
      block: u64
      raw: u64
      structure: u8
      sdSpec: u8
      dataAfterErase: bool
      security: u8
      busWidths: u8
      spec3: bool
      extendedSecurity: u8
      spec4: bool
      specExtension: u8
      commandSupport: u8
      vendorData: u32
      crcOk: bool
      supported: bool
    }
  | {
      kind: 'command'
      token: u64
      host: bool
      index: u8
      argument: u32
      crc: u8
      crcOk: bool | undefined
      endOk: bool
    }
  | {
      kind: 'cid'
      token: u64
      productName: [u8, u8, u8, u8, u8]
      manufacturer: u8
      oem: u16
      revision: u8
      serial: u32
      year: u16
      month: u8
      crcOk: bool
      endOk: bool
    }
  | {
      kind: 'csd'
      token: u64
      taac: u8
      nsac: u8
      transferSpeed: u8
      commandClasses: u16
      readPartial: bool
      writeMisalign: bool
      readMisalign: bool
      dsrImplemented: bool
      readCurrentMin: u8 | undefined
      readCurrentMax: u8 | undefined
      writeCurrentMin: u8 | undefined
      writeCurrentMax: u8 | undefined
      sizeMultiplier: u8 | undefined
      eraseSingleBlock: bool
      eraseSectorBlocks: u8
      writeProtectGroupSectors: u8 | undefined
      writeProtectGroupEnabled: bool | undefined
      writeSpeedFactor: u8
      writeBlockBits: u8
      writePartial: bool
      fileFormatGroup: bool
      copied: bool
      permanentWriteProtect: bool
      temporaryWriteProtect: bool
      fileFormat: u8
      registerCrc: u8
      structure: u8
      size: u32
      blocks: u64
      readBlockBits: u8
      supported: bool
      crcOk: bool
      endOk: bool
    }
  | { kind: 'data-begin'; block: u64; read: bool; command: u8; bytes: u16; lanes: u8 }
  | { kind: 'data'; block: u64; offset: u16; value: u8 }
  | { kind: 'data-crc'; block: u64; lane: u8; value: u16; expected: u16; ok: bool }
  | { kind: 'data-status'; block: u64; status: u8; accepted: bool; endOk: bool }
  | { kind: 'busy'; block: u64; ticks: u64 }
  | { kind: 'data-end'; block: u64; reason: End }
  | { kind: 'bus-width'; lanes: u8; applied: bool }
  | { kind: 'aborted-command'; token: u64; bits: u16 }
export const events = eventSchema<Event>('sd-native.events', 1).roles({
  scr: { role: 'data', errorWhen: { crcOk: false } },
  'bus-width': 'control',
  command: 'command',
  'card-status': { role: 'control', errorWhen: [{ crcOk: false }, { endOk: false }] },
  'status-field': 'control',
  'relative-address': { role: 'address', errorWhen: [{ crcOk: false }, { endOk: false }] },
  'interface-condition': {
    role: 'control',
    errorWhen: [{ crcOk: false }, { endOk: false }, { patternOk: false }, { voltageOk: false }],
  },
  ocr: 'control',
  cid: 'data',
  csd: 'data',
  'data-begin': 'framing',
  data: 'data',
  'data-crc': { role: 'check', errorWhen: { ok: false } },
  'data-status': 'handshake',
  busy: 'handshake',
  'data-end': 'framing',
  'aborted-command': 'framing',
})
export type Width = 'one' | 'four' | 'auto'
export interface Config {
  clk: Channel
  cmd: Channel
  dat0: Channel | undefined
  dat1: Channel | undefined
  dat2: Channel | undefined
  dat3: Channel | undefined
  width: Width
  blockBytes: u16
}
function highest(c: Config): u8 {
  return max(max(c.clk, c.cmd), max(max(c.dat0 ?? 0, c.dat1 ?? 0), max(c.dat2 ?? 0, c.dat3 ?? 0)))
}
function validate(c: Config): string | undefined {
  if (highest(c) >= 64) return 'sd-native.config.channel'
  if (c.blockBytes === 0 || c.blockBytes > 4096) return 'sd-native.config.blockBytes'
  if (
    (c.width === 'four' || c.width === 'auto') &&
    (c.dat0 === undefined || c.dat1 === undefined || c.dat2 === undefined || c.dat3 === undefined)
  )
    return 'sd-native.config.dataWidth'
  if (
    c.clk === c.cmd ||
    c.dat0 === c.clk ||
    c.dat0 === c.cmd ||
    c.dat1 === c.clk ||
    c.dat1 === c.cmd ||
    c.dat2 === c.clk ||
    c.dat2 === c.cmd ||
    c.dat3 === c.clk ||
    c.dat3 === c.cmd
  )
    return 'sd-native.config.duplicateChannel'
  if (c.dat0 !== undefined && (c.dat0 === c.dat1 || c.dat0 === c.dat2 || c.dat0 === c.dat3))
    return 'sd-native.config.duplicateChannel'
  if (c.dat1 !== undefined && (c.dat1 === c.dat2 || c.dat1 === c.dat3))
    return 'sd-native.config.duplicateChannel'
  if (c.dat2 !== undefined && c.dat2 === c.dat3) return 'sd-native.config.duplicateChannel'
  return undefined
}
function crc7(crc: u8, bit: u8): u8 {
  const top: u8 = ((crc >> 6) ^ bit) & 1
  return ((crc << 1) & 127) ^ (top !== 0 ? 9 : 0)
}
function crc16(crc: u16, bit: u8): u16 {
  const top: u8 = (((crc >> 15) ^ bit) & 1) as u8
  return (crc << 1) ^ (top !== 0 ? 0x1021 : 0)
}
type DataPhase = 'idle' | 'data' | 'crc' | 'end' | 'status' | 'busy'
function statusField(bit: u8): StatusField | undefined {
  switch (bit) {
    case 31:
      return 'out-of-range'
    case 30:
      return 'address-error'
    case 29:
      return 'block-length-error'
    case 28:
      return 'erase-sequence-error'
    case 27:
      return 'erase-parameter'
    case 26:
      return 'write-protect-violation'
    case 25:
      return 'card-locked'
    case 24:
      return 'lock-unlock-failed'
    case 23:
      return 'command-crc-error'
    case 22:
      return 'illegal-command'
    case 21:
      return 'ecc-failed'
    case 20:
      return 'controller-error'
    case 19:
      return 'general-error'
    case 16:
      return 'csd-overwrite'
    case 15:
      return 'write-protect-erase-skip'
    case 14:
      return 'ecc-disabled'
    case 13:
      return 'erase-reset'
    case 9:
      return 'current-state'
    case 8:
      return 'ready-for-data'
    case 6:
      return 'fx-event'
    case 5:
      return 'application-command'
    case 3:
      return 'authentication-sequence-error'
    default:
      return undefined
  }
}
function voltageRange(code: u8): Voltage {
  if (code === 1) return 'v27-36'
  if (code === 2) return 'reserved-low-voltage'
  if (code === 4 || code === 8) return 'reserved'
  return 'not-defined'
}
class SdNative extends LogicDecoder<Config, Event> {
  readonly watch = mask(this.config.clk)
  nextId: u64 = 1
  token: u64 = 0
  cmdBits: u16 = 0
  header: u8 = 0
  host = false
  index: u8 = 0
  cmdFirst: Tick = 0
  cmdIdle: u8 = 0
  cmdAligned = false
  lastCommand: u8 | undefined = undefined
  lastArgument: u32 = 0
  commandCrc: u8 = 0
  receivedCrc: u8 = 0
  a: u32 = 0
  b: u32 = 0
  c: u32 = 0
  d: u32 = 0
  long = false
  blockBytes: u16 = this.config.blockBytes
  activeWidth: u8 = this.config.width === 'four' ? 4 : 1
  appAccepted = false
  pendingWidth: u8 | undefined = undefined
  pendingWidthAt: Tick | undefined = undefined
  transferCommand: u8 | undefined = undefined
  transferBytes: u16 = this.config.blockBytes
  scrA: u32 = 0
  scrB: u32 = 0
  transferRead = false
  multiple = false
  dataReady = false
  dataPhase: DataPhase = 'idle'
  dataAligned = false
  block: u64 = 0
  dataFirst: Tick = 0
  byteFirst: Tick = 0
  dataBits: u32 = 0
  byteBits: u8 = 0
  byte: u8 = 0
  dataOffset: u16 = 0
  crc0: u16 = 0
  crc1: u16 = 0
  crc2: u16 = 0
  crc3: u16 = 0
  got0: u16 = 0
  got1: u16 = 0
  got2: u16 = 0
  got3: u16 = 0
  crcBits: u8 = 0
  statusBits: u8 = 0
  status: u8 = 0
  busyStart: Tick | undefined = undefined
  override hold(): Tick | undefined {
    let held: Tick | undefined = this.cmdBits > 0 ? this.cmdFirst : undefined
    if (this.transferCommand === 51 && this.dataPhase !== 'idle')
      held = held === undefined ? this.dataFirst : min(held, this.dataFirst)
    if (this.dataPhase === 'data' && this.byteBits > 0)
      held = held === undefined ? this.byteFirst : min(held, this.byteFirst)
    if (this.dataPhase === 'crc')
      held = held === undefined ? this.byteFirst : min(held, this.byteFirst)
    if (this.dataPhase === 'status' && this.statusBits > 0)
      held = held === undefined ? this.byteFirst : min(held, this.byteFirst)
    if (this.busyStart !== undefined)
      held = held === undefined ? this.busyStart : min(held, this.busyStart)
    if (this.pendingWidthAt !== undefined)
      held = held === undefined ? this.pendingWidthAt : min(held, this.pendingWidthAt)
    return held
  }
  async cut(tick: Tick): Promise<void> {
    if (this.cmdBits > 0)
      await this.out.emit(
        { kind: 'aborted-command', token: this.token, bits: this.cmdBits },
        at(tick),
      )
    if (this.dataPhase !== 'idle')
      await this.out.emit({ kind: 'data-end', block: this.block, reason: 'aborted' }, at(tick))
    this.activeWidth = this.config.width === 'four' ? 4 : 1
    this.appAccepted = false
    this.pendingWidth = undefined
    this.pendingWidthAt = undefined
    this.cmdBits = 0
    this.cmdAligned = false
    this.cmdIdle = 0
    this.lastCommand = undefined
    this.transferCommand = undefined
    this.dataReady = false
    this.dataPhase = 'idle'
    this.dataAligned = false
    this.busyStart = undefined
    this.byteBits = 0
  }
  override async onGap(range: Range): Promise<void> {
    await this.cut(range.start)
    this.blockBytes = this.config.blockBytes
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
  }
  async register(tick: Tick, ok: bool, endOk: bool): Promise<void> {
    const where = over(this.cmdFirst, tick + 1, tick)
    if (this.lastCommand === 2 || this.lastCommand === 10) {
      const date: u16 = ((this.d >> 8) & 0xfff) as u16
      await this.out.emit(
        {
          kind: 'cid',
          token: this.token,
          productName: [
            (this.a & 255) as u8,
            (this.b >> 24) as u8,
            ((this.b >> 16) & 255) as u8,
            ((this.b >> 8) & 255) as u8,
            (this.b & 255) as u8,
          ],
          manufacturer: (this.a >> 24) as u8,
          oem: ((this.a >> 8) & 0xffff) as u16,
          revision: (this.c >> 24) as u8,
          serial: ((this.c & 0xffffff) << 8) | (this.d >> 24),
          year: 2000 + (date >> 4),
          month: (date & 15) as u8,
          crcOk: ok,
          endOk,
        },
        where,
      )
    } else if (this.lastCommand === 9) {
      const structure: u8 = (this.a >> 30) as u8
      const readBlockBits: u8 = ((this.b >> 16) & 15) as u8
      let size: u32 = 0
      let blocks: u64 = 0
      if (structure === 1) {
        size = ((this.b & 63) << 16) | (this.c >> 16)
        blocks = (size + 1) * 1024
      } else if (structure === 0) {
        size = ((this.b & 0x3ff) << 2) | (this.c >> 30)
        const mult: u8 = ((this.c >> 15) & 7) as u8
        blocks = ((size + 1) * (1 << (mult + 2)) * (1 << readBlockBits)) / 512
      }
      await this.out.emit(
        {
          kind: 'csd',
          token: this.token,
          taac: (this.a >> 16) as u8,
          nsac: ((this.a >> 8) & 255) as u8,
          transferSpeed: (this.a & 255) as u8,
          commandClasses: (this.b >> 20) as u16,
          readPartial: (this.b & 0x8000) !== 0,
          writeMisalign: (this.b & 0x4000) !== 0,
          readMisalign: (this.b & 0x2000) !== 0,
          dsrImplemented: (this.b & 0x1000) !== 0,
          readCurrentMin: structure === 0 ? (((this.c >> 27) & 7) as u8) : undefined,
          readCurrentMax: structure === 0 ? (((this.c >> 24) & 7) as u8) : undefined,
          writeCurrentMin: structure === 0 ? (((this.c >> 21) & 7) as u8) : undefined,
          writeCurrentMax: structure === 0 ? (((this.c >> 18) & 7) as u8) : undefined,
          sizeMultiplier: structure === 0 ? (((this.c >> 15) & 7) as u8) : undefined,
          eraseSingleBlock: (this.c & 0x4000) !== 0,
          eraseSectorBlocks: (((this.c >> 7) & 127) + 1) as u8,
          writeProtectGroupSectors: structure === 0 ? (((this.c & 127) + 1) as u8) : undefined,
          writeProtectGroupEnabled: structure === 0 ? (this.d & 0x80000000) !== 0 : undefined,
          writeSpeedFactor: ((this.d >> 26) & 7) as u8,
          writeBlockBits: ((this.d >> 22) & 15) as u8,
          writePartial: (this.d & 0x200000) !== 0,
          fileFormatGroup: (this.d & 0x8000) !== 0,
          copied: (this.d & 0x4000) !== 0,
          permanentWriteProtect: (this.d & 0x2000) !== 0,
          temporaryWriteProtect: (this.d & 0x1000) !== 0,
          fileFormat: ((this.d >> 10) & 3) as u8,
          registerCrc: ((this.d >> 1) & 127) as u8,
          structure,
          size,
          blocks,
          readBlockBits,
          supported: structure <= 1,
          crcOk: ok,
          endOk,
        },
        where,
      )
    }
  }
  async response(tick: Tick, checked: bool | undefined, endOk: bool): Promise<void> {
    const command = this.lastCommand
    if (command === undefined) return
    const where = over(this.cmdFirst, tick + 1, tick)
    if (command === 41 && this.index === 63) {
      await this.out.emit(
        {
          kind: 'ocr',
          token: this.token,
          value: this.a,
          ready: (this.a & 0x80000000) !== 0,
          highCapacity: (this.a & 0x40000000) !== 0,
          voltageWindow: this.a & 0xffffff,
          endOk,
        },
        where,
      )
      return
    }
    if (this.index !== command) return
    if (command === 8) {
      const voltage: u8 = ((this.a >> 8) & 15) as u8
      const pattern: u8 = (this.a & 255) as u8
      await this.out.emit(
        {
          kind: 'interface-condition',
          token: this.token,
          voltage,
          voltageRange: voltageRange(voltage),
          pattern,
          expectedPattern: (this.lastArgument & 255) as u8,
          patternOk: pattern === (this.lastArgument & 255),
          voltageOk: voltage === ((this.lastArgument >> 8) & 15),
          reserved: this.a >> 12,
          crcOk: checked === true,
          endOk,
        },
        where,
      )
      return
    }
    if (command === 5 || command === 41) return
    let format: StatusFormat = 'r1'
    let value: u32 = this.a
    let relativeAddress: u16 | undefined = undefined
    if (command === 3) {
      format = 'r6'
      const rca: u16 = (this.a >> 16) as u16
      relativeAddress = rca
      const status: u16 = (this.a & 0xffff) as u16
      value =
        (((status & 0xc000) as u32) << 8) | (((status & 0x2000) as u32) << 6) | (status & 0x1fff)
      await this.out.emit(
        {
          kind: 'relative-address',
          token: this.token,
          address: rca,
          status,
          crcOk: checked === true,
          endOk,
        },
        where,
      )
    } else if (
      command === 7 ||
      command === 12 ||
      command === 28 ||
      command === 29 ||
      command === 38
    )
      format = 'r1b'
    await this.out.emit(
      {
        kind: 'card-status',
        token: this.token,
        format,
        value,
        relativeAddress,
        currentState: ((value >> 9) & 15) as u8,
        readyForData: (value & 256) !== 0,
        appCommand: (value & 32) !== 0,
        crcOk: checked === true,
        endOk,
      },
      where,
    )
    for (let bit: u8 = 0; bit < 32; bit += 1) {
      const field = statusField(bit)
      if (field === undefined) continue
      if (format === 'r6' && bit > 12 && bit !== 19 && bit !== 22 && bit !== 23) continue
      const fieldValue: u8 =
        bit === 9 ? (((value >> 9) & 15) as u8) : (((value >> (bit as u32)) & 1) as u8)
      await this.out.emit(
        { kind: 'status-field', token: this.token, field, value: fieldValue },
        where,
      )
    }
  }
  async cmdBit(tick: Tick, bit: u8): Promise<void> {
    if (this.cmdBits === 0) {
      if (bit !== 0) {
        if (this.cmdIdle < 8) this.cmdIdle += 1
        if (this.cmdIdle === 8) this.cmdAligned = true
        return
      }
      if (!this.cmdAligned) return
      this.token = this.nextId
      this.nextId += 1
      this.cmdFirst = tick
      this.cmdBits = 1
      this.header = 0
      this.commandCrc = 0
      this.receivedCrc = 0
      this.a = 0
      this.b = 0
      this.c = 0
      this.d = 0
      return
    }
    const position = this.cmdBits
    if (position < 8) {
      this.header = (this.header << 1) | bit
      this.commandCrc = crc7(this.commandCrc, bit)
      if (position === 7) {
        this.host = (this.header & 64) !== 0
        this.index = this.header & 63
        this.long =
          !this.host &&
          (this.lastCommand === 2 || this.lastCommand === 9 || this.lastCommand === 10)
        if (this.long) this.commandCrc = 0
      }
    } else if (this.long) {
      const at: u16 = position - 8
      if (at < 32) this.a = (this.a << 1) | bit
      else if (at < 64) this.b = (this.b << 1) | bit
      else if (at < 96) this.c = (this.c << 1) | bit
      else this.d = (this.d << 1) | bit
      if (at < 120) this.commandCrc = crc7(this.commandCrc, bit)
    } else if (position < 40) {
      this.a = (this.a << 1) | bit
      this.commandCrc = crc7(this.commandCrc, bit)
    } else if (position < 47) this.receivedCrc = (this.receivedCrc << 1) | bit
    this.cmdBits += 1
    if (this.cmdBits < (this.long ? 136 : 48)) return
    const endOk = bit !== 0
    if (this.long) {
      const received: u8 = ((this.d >> 1) & 127) as u8
      await this.register(tick, received === this.commandCrc, endOk)
    } else {
      let checked: bool | undefined = this.receivedCrc === this.commandCrc
      if (!this.host && (this.lastCommand === 41 || this.lastCommand === 5)) checked = undefined
      await this.out.emit(
        {
          kind: 'command',
          token: this.token,
          host: this.host,
          index: this.index,
          argument: this.a,
          crc: this.receivedCrc,
          crcOk: checked,
          endOk,
        },
        over(this.cmdFirst, tick + 1, tick),
      )
      if (!this.host) await this.response(tick, checked, endOk)
      if (this.host && checked === true && endOk) {
        const application = this.appAccepted
        this.pendingWidth = undefined
        this.pendingWidthAt = undefined
        if (this.index === 6 && application && (this.a === 0 || this.a === 2)) {
          this.pendingWidth = this.a === 2 ? 4 : 1
          this.pendingWidthAt = this.cmdFirst
        }
        this.appAccepted = false
        if (this.index === 0 && this.config.width === 'auto') this.activeWidth = 1
        this.lastCommand = this.index
        this.lastArgument = this.a
        if (this.index === 16 && this.a > 0 && this.a <= 4096) this.blockBytes = this.a as u16
        if (
          (this.index === 51 && application) ||
          this.index === 17 ||
          this.index === 18 ||
          this.index === 24 ||
          this.index === 25
        ) {
          this.transferBytes = this.index === 51 ? 8 : this.blockBytes
          this.transferCommand = this.index
          this.transferRead = this.index === 51 || this.index === 17 || this.index === 18
          this.multiple = this.index === 18 || this.index === 25
          this.dataReady = false
        }
        if (this.index === 12 || this.index === 0) {
          this.transferCommand = undefined
          this.dataReady = false
        }
      } else if (!this.host && endOk && checked === true && this.index === this.lastCommand) {
        const accepted = (this.a & 0xfff98008) === 0
        if (this.lastCommand === 55) this.appAccepted = accepted && (this.a & 32) !== 0
        const lanes = this.pendingWidth
        const first = this.pendingWidthAt
        this.pendingWidth = undefined
        this.pendingWidthAt = undefined
        if (lanes !== undefined) {
          if (first !== undefined) {
            if (this.lastCommand === 6 && accepted) {
              const applied = this.config.width === 'auto'
              if (applied) this.activeWidth = lanes
              await this.out.emit(
                { kind: 'bus-width', lanes, applied },
                over(first, tick + 1, tick),
              )
            }
          }
        }
        if (
          accepted &&
          this.transferCommand !== undefined &&
          this.lastCommand === this.transferCommand
        ) {
          this.dataReady = true
          this.dataAligned = false
        }
      }
    }
    this.cmdBits = 0
    this.cmdIdle = 0
    this.cmdAligned = true
  }
  async finishBlock(tick: Tick, reason: End): Promise<void> {
    await this.out.emit({ kind: 'data-end', block: this.block, reason }, one(tick))
    this.dataPhase = 'idle'
    this.dataAligned = false
    this.busyStart = undefined
    if (!this.multiple) this.dataReady = false
  }
  async dataBit(tick: Tick, levels: Levels): Promise<void> {
    const dat0 = this.config.dat0
    if (dat0 === undefined) return
    const lanes = this.activeWidth
    const bit0: u8 = fromBool(levels.high(dat0))
    const bit1: u8 = fromBool(levels.high(this.config.dat1 ?? dat0))
    const bit2: u8 = fromBool(levels.high(this.config.dat2 ?? dat0))
    const bit3: u8 = fromBool(levels.high(this.config.dat3 ?? dat0))
    const allHigh = lanes === 1 ? bit0 !== 0 : bit0 !== 0 && bit1 !== 0 && bit2 !== 0 && bit3 !== 0
    const allLow = lanes === 1 ? bit0 === 0 : bit0 === 0 && bit1 === 0 && bit2 === 0 && bit3 === 0
    if (this.dataPhase === 'idle') {
      if (allHigh) this.dataAligned = true
      const command = this.transferCommand
      if (command === undefined) return
      if (!this.dataReady || !this.dataAligned || !allLow) return
      this.block = this.nextId
      this.nextId += 1
      this.dataPhase = 'data'
      this.dataFirst = tick
      this.dataBits = 0
      this.byteBits = 0
      this.byte = 0
      this.dataOffset = 0
      this.scrA = 0
      this.scrB = 0
      this.crc0 = 0
      this.crc1 = 0
      this.crc2 = 0
      this.crc3 = 0
      this.got0 = 0
      this.got1 = 0
      this.got2 = 0
      this.got3 = 0
      await this.out.emit(
        {
          kind: 'data-begin',
          block: this.block,
          read: this.transferRead,
          command,
          bytes: this.transferBytes,
          lanes,
        },
        one(tick),
      )
      return
    }
    if (this.dataPhase === 'data') {
      if (this.byteBits === 0) this.byteFirst = tick
      this.crc0 = crc16(this.crc0, bit0)
      if (lanes === 4) {
        this.crc1 = crc16(this.crc1, bit1)
        this.crc2 = crc16(this.crc2, bit2)
        this.crc3 = crc16(this.crc3, bit3)
      }
      const group: u8 = lanes === 1 ? bit0 : bit0 | (bit1 << 1) | (bit2 << 2) | (bit3 << 3)
      this.byte = (this.byte << lanes) | group
      this.byteBits += lanes
      this.dataBits += lanes
      if (this.byteBits === 8) {
        if (this.transferCommand === 51) {
          if (this.dataOffset < 4) this.scrA = (this.scrA << 8) | this.byte
          else this.scrB = (this.scrB << 8) | this.byte
        }
        await this.out.emit(
          { kind: 'data', block: this.block, offset: this.dataOffset, value: this.byte },
          over(this.byteFirst, tick + 1, tick),
        )
        this.dataOffset += 1
        this.byteBits = 0
        this.byte = 0
      }
      if (this.dataBits === this.transferBytes * 8) {
        this.dataPhase = 'crc'
        this.crcBits = 0
        this.byteFirst = tick + 1
      }
      return
    }
    if (this.dataPhase === 'crc') {
      if (this.crcBits === 0) this.byteFirst = tick
      this.got0 = (this.got0 << 1) | bit0
      this.got1 = (this.got1 << 1) | bit1
      this.got2 = (this.got2 << 1) | bit2
      this.got3 = (this.got3 << 1) | bit3
      this.crcBits += 1
      if (this.crcBits === 16) {
        const where = over(this.byteFirst, tick + 1, tick)
        await this.out.emit(
          {
            kind: 'data-crc',
            block: this.block,
            lane: 0,
            value: this.got0,
            expected: this.crc0,
            ok: this.got0 === this.crc0,
          },
          where,
        )
        if (lanes === 4) {
          await this.out.emit(
            {
              kind: 'data-crc',
              block: this.block,
              lane: 1,
              value: this.got1,
              expected: this.crc1,
              ok: this.got1 === this.crc1,
            },
            where,
          )
          await this.out.emit(
            {
              kind: 'data-crc',
              block: this.block,
              lane: 2,
              value: this.got2,
              expected: this.crc2,
              ok: this.got2 === this.crc2,
            },
            where,
          )
          await this.out.emit(
            {
              kind: 'data-crc',
              block: this.block,
              lane: 3,
              value: this.got3,
              expected: this.crc3,
              ok: this.got3 === this.crc3,
            },
            where,
          )
        }
        this.dataPhase = 'end'
      }
      return
    }
    if (this.dataPhase === 'end') {
      if (!allHigh) {
        await this.finishBlock(tick, 'invalid-end')
        return
      }
      if (this.transferRead) {
        if (this.transferCommand === 51) {
          const a = this.scrA
          const b = this.scrB
          const crcOk =
            this.got0 === this.crc0 &&
            (lanes === 1 ||
              (this.got1 === this.crc1 && this.got2 === this.crc2 && this.got3 === this.crc3))
          await this.out.emit(
            {
              kind: 'scr',
              block: this.block,
              raw: ((a as u64) << 32) | (b as u64),
              structure: (a >> 28) as u8,
              sdSpec: ((a >> 24) & 15) as u8,
              dataAfterErase: ((a >> 23) & 1) !== 0,
              security: ((a >> 20) & 7) as u8,
              busWidths: ((a >> 16) & 15) as u8,
              spec3: ((a >> 15) & 1) !== 0,
              extendedSecurity: ((a >> 11) & 15) as u8,
              spec4: ((a >> 10) & 1) !== 0,
              specExtension: ((a >> 6) & 15) as u8,
              commandSupport: (a & 15) as u8,
              vendorData: b,
              crcOk,
              supported: a >> 28 === 0,
            },
            over(this.dataFirst, tick + 1, tick),
          )
        }

        await this.finishBlock(tick, 'complete')
        return
      }
      this.dataPhase = 'status'
      this.statusBits = 0
      this.status = 0
      return
    }
    if (this.dataPhase === 'status') {
      if (this.statusBits === 0) {
        if (bit0 !== 0) return
        this.byteFirst = tick
        this.statusBits = 1
        this.status = 0
        return
      }
      this.status = (this.status << 1) | bit0
      this.statusBits += 1
      if (this.statusBits === 5) {
        const status: u8 = (this.status >> 1) & 7
        await this.out.emit(
          {
            kind: 'data-status',
            block: this.block,
            status,
            accepted: status === 2,
            endOk: bit0 !== 0,
          },
          over(this.byteFirst, tick + 1, tick),
        )
        this.dataPhase = 'busy'
        this.busyStart = undefined
      }
      return
    }
    if (this.dataPhase === 'busy') {
      if (bit0 === 0) {
        if (this.busyStart === undefined) this.busyStart = tick
        return
      }
      const start = this.busyStart ?? tick
      await this.out.emit(
        { kind: 'busy', block: this.block, ticks: tick - start },
        over(start, tick + 1, tick),
      )
      await this.finishBlock(tick, 'complete')
    }
  }
  override async run(): Promise<void> {
    await this.signal.sample()
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      if (edge.levels.high(this.config.clk)) {
        await this.cmdBit(edge.tick, fromBool(edge.levels.high(this.config.cmd)))
        await this.dataBit(edge.tick, edge.levels)
      }
    }
  }
}
export const sdNative = defineLogicDecoder({
  name: 'sd-native',
  category: 'bus',
  config: {
    defaults: {
      clk: 0,
      cmd: 1,
      dat0: undefined,
      dat1: undefined,
      dat2: undefined,
      dat3: undefined,
      width: 'one',
      blockBytes: 512,
    },
    validate,
  },
  requires: (config) => ({ channels: atLeast(highest(config) + 1) }),
  output: events,
  props: () => ({ sdr: true, readWrite: true }),

  locales: {
    'en-US': {
      name: 'SD card (commands and data)',
      summary: 'SD SDR commands, CID/CSD, read/write CRC and card busy periods',
      fields: {
        clk: 'Clock channel',
        cmd: 'Cmd',
        dat0: 'Dat0',
        dat1: 'Dat1',
        dat2: 'Dat2',
        dat3: 'Dat3',
        width: 'Width',
        blockBytes: 'Block Bytes',
      },
      events: {
        'card-status': 'Card status',
        'status-field': 'Card status field',
        'relative-address': 'Published relative address',
        'interface-condition': 'Card interface condition',
        ocr: 'Operating conditions',
        scr: 'SCR register',
        command: 'Command',
        cid: 'Cid',
        csd: 'Csd',
        'data-begin': 'Data begin',
        data: 'Data',
        'data-crc': 'Data crc',
        'data-status': 'Data status',
        busy: 'Busy',
        'data-end': 'Data end',
        'bus-width': 'Bus width',
        'aborted-command': 'Aborted command',
      },
      eventFields: {
        format: 'Response format',
        relativeAddress: 'Relative card address',
        currentState: 'Current card state',
        readyForData: 'Ready for data',
        appCommand: 'Application command accepted',
        field: 'Status field',
        address: 'Relative card address',
        voltage: 'Voltage code',
        voltageRange: 'Accepted voltage range',
        pattern: 'Echoed check pattern',
        expectedPattern: 'Expected check pattern',
        patternOk: 'Check pattern matches',
        voltageOk: 'Voltage matches request',
        reserved: 'Reserved bits',
        ready: 'Power-up complete',
        highCapacity: 'High-capacity card',
        voltageWindow: 'Supply-voltage window mask',
        taac: 'TAAC access-time code',
        nsac: 'NSAC clock-count code',
        transferSpeed: 'Maximum transfer-rate code',
        commandClasses: 'Command class mask',
        readPartial: 'Partial read blocks',
        writeMisalign: 'Write block misalignment',
        readMisalign: 'Read block misalignment',
        dsrImplemented: 'DSR implemented',
        readCurrentMin: 'Minimum-voltage read-current code',
        readCurrentMax: 'Maximum-voltage read-current code',
        writeCurrentMin: 'Minimum-voltage write-current code',
        writeCurrentMax: 'Maximum-voltage write-current code',
        sizeMultiplier: 'Capacity multiplier code',
        eraseSingleBlock: 'Single-block erase',
        eraseSectorBlocks: 'Erase sector write blocks',
        writeProtectGroupSectors: 'Write-protect group sectors',
        writeProtectGroupEnabled: 'Group write protection',
        writeSpeedFactor: 'Write/read time factor code',
        writeBlockBits: 'Write block length exponent',
        writePartial: 'Partial write blocks',
        fileFormatGroup: 'File format group',
        copied: 'Copy flag',
        permanentWriteProtect: 'Permanent write protection',
        temporaryWriteProtect: 'Temporary write protection',
        fileFormat: 'File format code',
        registerCrc: 'Register CRC7',
        productName: 'Product name bytes',
        raw: 'Raw SCR',
        sdSpec: 'SD specification code',
        dataAfterErase: 'Erased data is one',
        security: 'Security version',
        busWidths: 'Supported bus widths',
        spec3: 'Version 3 or later',
        extendedSecurity: 'Extended security',
        spec4: 'Version 4 or later',
        specExtension: 'Specification extension',
        commandSupport: 'Supported commands',
        vendorData: 'Manufacturer data',
        token: 'Token',
        host: 'Host',
        index: 'Index',
        argument: 'Argument',
        crc: 'CRC',
        crcOk: 'CRC valid',
        endOk: 'End Ok',
        manufacturer: 'Manufacturer',
        oem: 'OEM',
        revision: 'Revision',
        serial: 'Serial',
        year: 'Year',
        month: 'Month',
        structure: 'Structure',
        size: 'Size',
        blocks: 'Blocks',
        readBlockBits: 'Read Block Bits',
        supported: 'Supported',
        block: 'Block',
        read: 'Read',
        command: 'Command',
        bytes: 'Bytes',
        lanes: 'Lanes',
        offset: 'Offset',
        value: 'Value',
        lane: 'Lane',
        expected: 'Expected',
        ok: 'Ok',
        status: 'Status',
        accepted: 'Accepted',
        ticks: 'Ticks',
        reason: 'Reason',
        applied: 'Applied',
        bits: 'Bits',
      },
      values: {
        width: { one: 'One data line', four: 'Four data lines', auto: 'Follow accepted ACMD6' },
      },
    },
    'zh-CN': {
      name: 'SD 卡（命令与数据）',
      summary: '解析SD SDR命令、CID/CSD、读写CRC及卡忙时段',
      fields: {
        clk: '时钟通道',
        cmd: 'CMD通道',
        dat0: 'DAT0通道',
        dat1: 'DAT1通道',
        dat2: 'DAT2通道',
        dat3: 'DAT3通道',
        width: '数据线数',
        blockBytes: '块字节数',
      },
      events: {
        'card-status': '卡状态',
        'status-field': '卡状态字段',
        'relative-address': '公布的相对地址',
        'interface-condition': '卡接口条件',
        ocr: '操作条件',
        scr: 'SCR配置寄存器',
        command: '命令',
        cid: 'CID标识',
        csd: 'CSD配置',
        'data-begin': '数据开始',
        data: '数据通道',
        'data-crc': '数据CRC',
        'data-status': '数据状态',
        busy: '卡忙',
        'data-end': '数据结束',
        'bus-width': '总线宽度',
        'aborted-command': '命令中断',
      },
      eventFields: {
        format: '应答格式',
        relativeAddress: '相对卡地址',
        currentState: '当前卡状态',
        readyForData: '可传输数据',
        appCommand: '已接受应用命令',
        field: '状态字段',
        address: '相对卡地址',
        voltage: '电压编码',
        voltageRange: '接受的电压范围',
        pattern: '回显校验模式',
        expectedPattern: '预期校验模式',
        patternOk: '校验模式匹配',
        voltageOk: '电压与请求匹配',
        reserved: '保留位',
        ready: '上电完成',
        highCapacity: '高容量卡',
        voltageWindow: '供电电压窗口掩码',
        taac: 'TAAC访问时间编码',
        nsac: 'NSAC时钟计数编码',
        transferSpeed: '最大传输速率编码',
        commandClasses: '命令类别掩码',
        readPartial: '支持部分读块',
        writeMisalign: '允许写块地址未对齐',
        readMisalign: '允许读块地址未对齐',
        dsrImplemented: '实现DSR寄存器',
        readCurrentMin: '最低电压读取电流编码',
        readCurrentMax: '最高电压读取电流编码',
        writeCurrentMin: '最低电压写入电流编码',
        writeCurrentMax: '最高电压写入电流编码',
        sizeMultiplier: '容量倍数编码',
        eraseSingleBlock: '支持单块擦除',
        eraseSectorBlocks: '每擦除扇区的写块数',
        writeProtectGroupSectors: '每写保护组的扇区数',
        writeProtectGroupEnabled: '支持组写保护',
        writeSpeedFactor: '写入与读取时间倍数编码',
        writeBlockBits: '写块长度指数',
        writePartial: '支持部分写块',
        fileFormatGroup: '文件格式组',
        copied: '副本标志',
        permanentWriteProtect: '永久写保护',
        temporaryWriteProtect: '临时写保护',
        fileFormat: '文件格式编码',
        registerCrc: '寄存器CRC7',
        productName: '产品名字节',
        raw: '原始SCR',
        sdSpec: 'SD规范版本编码',
        dataAfterErase: '擦除后数据为一',
        security: '安全规范版本',
        busWidths: '支持的数据线宽度',
        spec3: '支持3.0及以上版本',
        extendedSecurity: '扩展安全能力',
        spec4: '支持4.0及以上版本',
        specExtension: '规范扩展版本',
        commandSupport: '支持的命令',
        vendorData: '制造商数据',
        token: '令牌号',
        host: '主机发送',
        index: '序号',
        argument: '参数',
        crc: 'CRC',
        crcOk: 'CRC有效',
        endOk: '结束位有效',
        manufacturer: '制造商',
        oem: 'OEM',
        revision: '版本',
        serial: '序列号',
        year: '年份',
        month: '月份',
        structure: '结构版本',
        size: '容量字段',
        blocks: '512字节块数',
        readBlockBits: '读块位数',
        supported: '支持的格式',
        block: '块编号',
        read: '读取',
        command: '命令',
        bytes: '字节数',
        lanes: '数据线数',
        offset: '偏移',
        value: '数值',
        lane: '数据线',
        expected: '预期值',
        ok: '正确',
        status: '状态',
        accepted: '已接受',
        ticks: '采样周期数',
        reason: '结束原因',
        applied: '已应用',
        bits: '位数',
      },
      values: { width: { one: '一线', four: '四线', auto: '跟随ACMD6' } },
    },
  },
  decoder: SdNative,
})
