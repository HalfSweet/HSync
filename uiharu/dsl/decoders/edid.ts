// SPDX-License-Identifier: Apache-2.0

/** EDID 1.x. Implemented from VESA E-EDID Release A Revision2 (1.4), §§3.1–3.11, primary authored standard mirrored publicly. Original implementation from primary tables; the pinned reference was consulted only for scope inventory.
 * EDID offset pointers persist across STOP; E-DDC segments reset on NACK/STOP.
 * Fixed scalar descriptor/CTA state and at most a 128-byte checksum provenance hold; no payload array.
 */
import type { ItemOf, Provenance, Tick, Transaction, bool, u16, u32, u64, u8 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, join } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'

type BusItem = ItemOf<typeof i2cEvents>
export type OverlayEnd = 'complete' | 'incomplete' | 'aborted' | 'nack'
export type Field =
  | 'red-x-1024'
  | 'red-y-1024'
  | 'green-x-1024'
  | 'green-y-1024'
  | 'blue-x-1024'
  | 'blue-y-1024'
  | 'white-x-1024'
  | 'white-y-1024'
  | 'horizontal-front-porch'
  | 'horizontal-sync-width'
  | 'vertical-front-porch'
  | 'vertical-sync-width'
  | 'horizontal-millimeters'
  | 'vertical-millimeters'
  | 'horizontal-border'
  | 'vertical-border'
  | 'interlaced'
  | 'stereo-code'
  | 'sync-format'
  | 'sync-flags'
  | 'minimum-vertical-hz'
  | 'maximum-vertical-hz'
  | 'minimum-horizontal-khz'
  | 'maximum-horizontal-khz'
  | 'maximum-pixel-clock-hz'
  | 'range-timing-support'
  | 'extension-revision'
  | 'cta-dtd-offset'
  | 'cta-flags'
  | 'cta-native-dtd-count'
  | 'cta-vendor-oui'
  | 'cta-speaker-mask'
  | 'cta-extended-tag'
  | 'checksum'
  | 'header-byte'
  | 'manufacturer-code'
  | 'product-code'
  | 'serial-number'
  | 'manufacture-week'
  | 'manufacture-year'
  | 'version'
  | 'revision'
  | 'video-input'
  | 'horizontal-centimeters'
  | 'vertical-centimeters'
  | 'gamma-code'
  | 'features'
  | 'extension-count'
  | 'extension-tag'
  | 'pixel-clock-hz'
  | 'horizontal-active'
  | 'horizontal-blanking'
  | 'vertical-active'
  | 'vertical-blanking'
  | 'descriptor-tag'
  | 'descriptor-byte'
  | 'standard-horizontal'
  | 'standard-vertical'
  | 'standard-refresh-hz'
  | 'standard-aspect-code'
  | 'standard-unused'
  | 'monitor-name-byte'
  | 'monitor-serial-byte'
  | 'monitor-text-byte'
  | 'additional-color-point-byte'
  | 'additional-standard-timing-byte'
  | 'register'
export type DeviceEvent =
  | {
      kind: 'begin'
      op: u64
      tx: u64
      read: bool
      ack: bool
      segment: u8 | undefined
    }
  | { kind: 'pointer'; op: u64; register: u8; ack: bool }
  | {
      kind: 'byte'
      op: u64
      register: u8 | undefined
      absolute: u32 | undefined
      value: u8
      read: bool
      ack: bool
    }
  | {
      kind: 'field'
      op: u64
      register: u8
      field: Field
      value: u32
      negative: bool
      valid: bool
      block: u16 | undefined
    }
  | {
      kind: 'timing'
      op: u64
      register: u8
      width: u16
      height: u16
      refresh: u8
      interlaced: bool
    }
  | { kind: 'segment'; tx: u64; value: u8; ack: bool; valid: bool }
  | {
      kind: 'cta-block'
      op: u64
      register: u8
      tag: u8
      length: u8
      valid: bool
    }
  | { kind: 'cta-video'; op: u64; register: u8; code: u8; native: bool }
  | {
      kind: 'cta-audio'
      op: u64
      register: u8
      format: u8
      channels: u8
      rates: u8
      detail: u8
      valid: bool
    }
  | { kind: 'end'; op: u64; reason: OverlayEnd }
export const deviceEvents = eventSchema<DeviceEvent>('edid.events', 1).roles({
  begin: { role: 'address', errorWhen: { ack: false } },
  pointer: 'address',
  byte: 'data',
  field: { role: 'data', errorWhen: { valid: false } },
  end: 'framing',
  timing: 'data',
  segment: { role: 'address', errorWhen: { valid: false } },
  'cta-block': { role: 'control', errorWhen: { valid: false } },
  'cta-video': 'data',
  'cta-audio': { role: 'data', errorWhen: { valid: false } },
})
export interface DeviceConfig {
  address: u16
  segmentAddress: u16
  initialSegment: u8 | undefined
}
function validate(config: DeviceConfig): string | undefined {
  if (
    config.address > 127 ||
    config.segmentAddress > 127 ||
    config.address === config.segmentAddress
  )
    return 'decode.config.address'
  if (config.initialSegment !== undefined && config.initialSegment > 127)
    return 'edid.config.segment'
  return undefined
}

class Device extends BusDecoder<DeviceConfig, BusItem, DeviceEvent, OverlayEnd> {
  pointer: u8 | undefined = undefined
  segment: u8 | undefined = this.config.initialSegment
  extension: u8 | undefined = undefined
  extensionRevision: u8 | undefined = undefined
  dtdStart: u8 | undefined = undefined
  dataRemaining: u8 = 0
  dataTag: u8 = 0
  dataIndex: u8 = 0
  dataAt: Provenance | undefined = undefined
  audioA: u8 = 0
  audioB: u8 = 0
  vendor: u32 = 0
  nextOp: u64 = 1
  open = false
  currentOp: u64 = 0
  half: u8 | undefined = undefined
  halfAt: Provenance | undefined = undefined
  checksum: u16 = 0
  covered: u16 = 0
  checksumAt: Provenance | undefined = undefined
  serial: u32 = 0
  serialSeen: u8 = 0
  detailed = false
  descriptorKnown = false
  descriptorTag: u8 | undefined = undefined
  revision: u8 | undefined = undefined
  lowA: u8 = 0
  lowB: u8 = 0
  lowC: u8 = 0
  redGreen: u8 | undefined = undefined
  blueWhite: u8 | undefined = undefined
  rangeOffsets: u8 | undefined = undefined
  rangeAt: Provenance | undefined = undefined
  redGreenAt: Provenance | undefined = undefined
  blueWhiteAt: Provenance | undefined = undefined
  override hold(): Tick | undefined {
    const first = this.checksumAt
    if (first !== undefined) return first.span.start
    const rg = this.redGreenAt
    if (rg !== undefined) return rg.span.start
    const bw = this.blueWhiteAt
    if (bw !== undefined) return bw.span.start
    const rangeAt = this.rangeAt
    if (rangeAt !== undefined) return rangeAt.span.start
    const dataAt = this.dataAt
    if (dataAt !== undefined) return dataAt.span.start
    const half = this.halfAt
    if (half !== undefined) return half.span.start
    return undefined
  }
  override onGap(): void {
    this.pointer = undefined
    this.segment = undefined
    this.extension = undefined
    this.extensionRevision = undefined
    this.dtdStart = undefined
    this.dataAt = undefined
    this.dataRemaining = 0
    this.half = undefined
    this.halfAt = undefined
    this.checksumAt = undefined
    this.covered = 0
    this.serialSeen = 0
    this.detailed = false
    this.descriptorKnown = false
    this.descriptorTag = undefined
    this.revision = undefined
    this.redGreen = undefined
    this.blueWhite = undefined
    this.rangeOffsets = undefined
    this.rangeAt = undefined
    this.redGreenAt = undefined
    this.blueWhiteAt = undefined
  }
  async close(reason: OverlayEnd, where: Provenance): Promise<void> {
    let ending = reason
    if (this.open) {
      const op = this.currentOp
      if (
        ending === 'complete' &&
        (this.halfAt !== undefined ||
          this.redGreenAt !== undefined ||
          this.blueWhiteAt !== undefined ||
          this.rangeAt !== undefined ||
          this.dataAt !== undefined)
      )
        ending = 'incomplete'
      this.half = undefined
      this.halfAt = undefined
      this.checksumAt = undefined
      this.serialSeen = 0
      this.detailed = false
      this.descriptorKnown = false
      this.descriptorTag = undefined
      this.redGreen = undefined
      this.blueWhite = undefined
      this.rangeOffsets = undefined
      this.rangeAt = undefined
      this.redGreenAt = undefined
      this.blueWhiteAt = undefined
      this.covered = 0
      this.extension = undefined
      this.extensionRevision = undefined
      this.dtdStart = undefined
      this.dataAt = undefined
      this.dataRemaining = 0
      this.open = false
      await this.out.emit({ kind: 'end', op, reason: ending }, where)
    }
  }
  override async recover(reason: OverlayEnd, where: Provenance): Promise<void> {
    this.pointer = undefined
    await this.close(reason, where)
  }
  async field(
    op: u64,
    register: u8,
    field: Field,
    value: u32,
    negative: bool,
    valid: bool,
    where: Provenance,
  ): Promise<void> {
    const segment = this.segment
    let block: u16 | undefined = undefined
    if (segment !== undefined) block = (segment as u16) * 2 + (register >= 128 ? 1 : 0)
    await this.out.emit(
      { kind: 'field', op, register, field, value, negative, valid, block },
      where,
    )
  }
  async interpret(op: u64, register: u8, byte: u8, read: bool, where: Provenance): Promise<void> {
    if (!read) return
    const offset: u8 = register & 127
    if (offset === 0) {
      this.checksum = 0
      this.covered = 0
      this.checksumAt = where
    }
    if (this.checksumAt !== undefined && offset === this.covered) {
      this.checksum = (this.checksum + byte) & 255
      this.covered += 1
      if (this.covered === 128) {
        const at = join(this.checksumAt, where)
        this.checksumAt = undefined
        await this.field(op, register, 'checksum', this.checksum, false, this.checksum === 0, at)
      }
    } else {
      this.checksumAt = undefined
      this.covered = 0
    }
    const segment = this.segment
    if (segment === undefined) return
    if (segment !== 0 || register >= 128) {
      await this.extensionByte(op, register, byte, where)
      return
    }
    if (register < 8) {
      await this.field(
        op,
        register,
        'header-byte',
        byte,
        false,
        byte === (register === 0 || register === 7 ? 0 : 255),
        where,
      )
      return
    }
    if (register === 8 || register === 10) {
      this.half = byte
      this.halfAt = where
      return
    }
    if ((register === 9 || register === 11) && this.half !== undefined) {
      const value: u16 = register === 9 ? (this.half << 8) | byte : (byte << 8) | this.half
      const at = join(this.halfAt!, where)
      this.half = undefined
      this.halfAt = undefined
      await this.field(
        op,
        register - 1,
        register === 9 ? 'manufacturer-code' : 'product-code',
        value,
        false,
        true,
        at,
      )
      return
    }
    if (register === 12) {
      this.serial = byte
      this.serialSeen = 1
      this.half = byte
      this.halfAt = where
      return
    }
    if (register >= 13 && register <= 15 && this.serialSeen === register - 12) {
      this.serial = this.serial | (byte << (this.serialSeen * 8))
      this.serialSeen += 1
      if (register === 15) {
        const at = join(this.halfAt!, where)
        this.half = undefined
        this.halfAt = undefined
        this.serialSeen = 0
        await this.field(op, 12, 'serial-number', this.serial, false, true, at)
      }
      return
    }
    let field: Field = 'register'
    switch (register) {
      case 16:
        field = 'manufacture-week'
        break
      case 17:
        await this.field(op, register, 'manufacture-year', byte + 1990, false, true, where)
        return
      case 18:
        field = 'version'
        break
      case 19:
        field = 'revision'
        break
      case 20:
        field = 'video-input'
        break
      case 21:
        field = 'horizontal-centimeters'
        break
      case 22:
        field = 'vertical-centimeters'
        break
      case 23:
        field = 'gamma-code'
        break
      case 24:
        field = 'features'
        break
      case 126:
        field = 'extension-count'
        break
      default:
        break
    }
    if (register === 19) this.revision = byte
    if (register === 25) {
      this.redGreen = byte
      this.redGreenAt = where
      return
    }
    if (register === 26) {
      this.blueWhite = byte
      this.blueWhiteAt = where
      return
    }
    if (register >= 27 && register <= 34) {
      const position: u8 = register - 27
      let component: Field = 'red-x-1024'
      if (position === 1) component = 'red-y-1024'
      else if (position === 2) component = 'green-x-1024'
      else if (position === 3) component = 'green-y-1024'
      else if (position === 4) component = 'blue-x-1024'
      else if (position === 5) component = 'blue-y-1024'
      else if (position === 6) component = 'white-x-1024'
      else if (position === 7) component = 'white-y-1024'
      const packed = position < 4 ? this.redGreen : this.blueWhite
      const packedAt = position < 4 ? this.redGreenAt : this.blueWhiteAt
      if (packed !== undefined && packedAt !== undefined) {
        const shift: u8 = 6 - (position & 3) * 2
        await this.field(
          op,
          register,
          component,
          ((byte as u32) << 2) | ((packed >> (shift as u64)) & 3),
          false,
          true,
          join(packedAt, where),
        )
      }
      if (position === 3) {
        this.redGreen = undefined
        this.redGreenAt = undefined
      }
      if (position === 7) {
        this.blueWhite = undefined
        this.blueWhiteAt = undefined
      }
      return
    }
    if (register >= 35 && register <= 37) {
      for (let bit = 0; bit < 8; bit += 1) {
        if ((byte & (128 >> (bit as u64))) === 0) continue
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-type-assertion -- The DSL loop index must narrow explicitly to the bounded timing code.
        const code: u8 = (((register - 35) as u16) * 8 + (bit as u16)) as u8
        let width: u16 = 0
        let height: u16 = 0
        let refresh: u8 = 0
        let interlaced = false
        if (code === 0 || code === 1) {
          width = 720
          height = 400
          refresh = code === 0 ? 70 : 88
        } else if (code >= 2 && code <= 5) {
          width = 640
          height = 480
          refresh = code === 2 ? 60 : code === 3 ? 67 : code === 4 ? 72 : 75
        } else if (code === 6 || code === 7 || code === 8 || code === 9) {
          width = 800
          height = 600
          refresh = code === 6 ? 56 : code === 7 ? 60 : code === 8 ? 72 : 75
        } else if (code === 10) {
          width = 832
          height = 624
          refresh = 75
        } else if (code >= 11 && code <= 14) {
          width = 1024
          height = 768
          refresh = code === 11 ? 87 : code === 12 ? 60 : code === 13 ? 70 : 75
          interlaced = code === 11
        } else if (code === 15) {
          width = 1280
          height = 1024
          refresh = 75
        } else if (code === 16) {
          width = 1152
          height = 870
          refresh = 75
        }
        if (width !== 0)
          await this.out.emit(
            {
              kind: 'timing',
              op,
              register,
              width,
              height,
              refresh,
              interlaced,
            },
            where,
          )
      }
      return
    }

    if (register >= 38 && register <= 53) {
      if ((register & 1) === 0) {
        this.half = byte
        this.halfAt = where
        return
      }
      const first = this.half
      if (first === undefined) return
      const placed = join(this.halfAt!, where)
      this.half = undefined
      this.halfAt = undefined
      if (first === 1 && byte === 1) {
        await this.field(op, register - 1, 'standard-unused', 1, false, true, placed)
        return
      }
      const horizontal: u32 = (first + 31) * 8
      const aspect: u8 = byte >> 6
      let vertical: u32 = 0
      let known = true
      if (aspect === 0) {
        if (this.revision === undefined) known = false
        else vertical = this.revision >= 3 ? (horizontal * 10) / 16 : horizontal
      } else if (aspect === 1) vertical = (horizontal * 3) / 4
      else if (aspect === 2) vertical = (horizontal * 4) / 5
      else vertical = (horizontal * 9) / 16
      await this.field(op, register - 1, 'standard-horizontal', horizontal, false, true, placed)
      await this.field(op, register - 1, 'standard-vertical', vertical, false, known, placed)
      await this.field(
        op,
        register - 1,
        'standard-refresh-hz',
        (byte & 63) + 60,
        false,
        true,
        placed,
      )
      await this.field(op, register - 1, 'standard-aspect-code', aspect, false, true, placed)
      return
    }
    if (register >= 54 && register < 126) {
      await this.descriptor(op, register, (register - 54) % 18, byte, where)
      return
    }

    await this.field(op, register, field, byte, false, true, where)
  }
  async extensionByte(op: u64, register: u8, byte: u8, where: Provenance): Promise<void> {
    const offset: u8 = register & 127
    if (offset === 0) {
      this.extension = byte
      this.extensionRevision = undefined
      this.dtdStart = undefined
      this.dataRemaining = 0
      this.dataAt = undefined
      this.detailed = false
      this.descriptorKnown = false
      await this.field(op, register, 'extension-tag', byte, false, true, where)
      return
    }
    if (this.extension !== 2) return
    if (offset === 1) {
      this.extensionRevision = byte
      await this.field(
        op,
        register,
        'extension-revision',
        byte,
        false,
        byte >= 1 && byte <= 3,
        where,
      )
      return
    }
    if (offset === 2) {
      this.dtdStart = byte === 0 ? 0 : byte >= 4 && byte <= 127 ? byte : undefined
      await this.field(
        op,
        register,
        'cta-dtd-offset',
        byte,
        false,
        byte === 0 || (byte >= 4 && byte <= 127),
        where,
      )
      return
    }
    if (offset === 3) {
      await this.field(op, register, 'cta-flags', byte >> 4, false, true, where)
      await this.field(op, register, 'cta-native-dtd-count', byte & 15, false, true, where)
      return
    }
    const start = this.dtdStart
    if (start === undefined) return
    if (start === 0 || offset === 127) return
    if (offset < start) {
      if (this.extensionRevision !== 3) return
      if (this.dataRemaining === 0) {
        const tag: u8 = byte >> 5
        const length: u8 = byte & 31
        const valid =
          (offset as u16) + (1 as u16) + (length as u16) <= (start as u16) &&
          tag !== 0 &&
          tag !== 6 &&
          (tag !== 1 || length % 3 === 0) &&
          (tag !== 3 || length >= 3) &&
          (tag !== 4 || length >= 3) &&
          (tag !== 7 || length >= 1)
        await this.out.emit({ kind: 'cta-block', op, register, tag, length, valid }, where)
        if (!valid) {
          this.dtdStart = undefined
          return
        }
        this.dataRemaining = length
        this.dataTag = tag
        this.dataIndex = 0
        this.vendor = 0
        this.dataAt = length === 0 ? undefined : where
        return
      }
      const index = this.dataIndex
      const tag = this.dataTag
      const first = this.dataAt
      this.dataIndex += 1
      this.dataRemaining -= 1
      if (first !== undefined) {
        const placed = join(first, where)
        if (tag === 2)
          await this.out.emit(
            {
              kind: 'cta-video',
              op,
              register,
              code: byte & 127,
              native: (byte & 128) !== 0,
            },
            placed,
          )
        else if (tag === 1) {
          if (index % 3 === 0) this.audioA = byte
          else if (index % 3 === 1) this.audioB = byte
          else
            await this.out.emit(
              {
                kind: 'cta-audio',
                op,
                register,
                format: (this.audioA >> 3) & 15,
                channels: (this.audioA & 7) + 1,
                rates: this.audioB & 127,
                detail: byte,
                valid:
                  (this.audioA & 128) === 0 &&
                  (this.audioA & 120) !== 0 &&
                  (this.audioB & 128) === 0,
              },
              placed,
            )
        } else if (tag === 3 || tag === 4) {
          if (index < 3) this.vendor |= (byte as u32) << ((index as u64) * 8)
          if (index === 2)
            await this.field(
              op,
              register,
              tag === 3 ? 'cta-vendor-oui' : 'cta-speaker-mask',
              this.vendor,
              false,
              true,
              placed,
            )
        } else if (tag === 7 && index === 0)
          await this.field(op, register, 'cta-extended-tag', byte, false, true, placed)
      }
      if (this.dataRemaining === 0) this.dataAt = undefined
      return
    }
    const index: u8 = (offset - start) % 18
    if ((offset as u16) + (18 as u16) - (index as u16) > (127 as u16)) return
    await this.descriptor(op, register, index, byte, where)
    if (index === 1 && !this.detailed) this.dtdStart = undefined
  }
  async descriptor(op: u64, register: u8, index: u8, byte: u8, where: Provenance): Promise<void> {
    let field: Field = 'register'

    if (index === 0) {
      this.half = byte
      this.halfAt = where
      this.detailed = false
      this.descriptorKnown = false
      this.descriptorTag = undefined
      return
    }
    if (index === 1 && this.half !== undefined) {
      const clock: u16 = (byte << 8) | this.half
      const at = join(this.halfAt!, where)
      this.half = undefined
      this.halfAt = undefined
      this.detailed = clock !== 0
      this.descriptorKnown = clock === 0
      await this.field(op, register - 1, 'pixel-clock-hz', clock * 10000, false, true, at)
      return
    }
    if (this.detailed) {
      if (index === 2 || index === 5) {
        this.lowA = byte
        this.halfAt = where
        return
      }
      if (index === 3 || index === 6) {
        this.lowB = byte
        return
      }
      if (index === 4 || index === 7) {
        if (this.halfAt !== undefined) {
          const at = join(this.halfAt, where)
          this.halfAt = undefined
          await this.field(
            op,
            register,
            index === 4 ? 'horizontal-active' : 'vertical-active',
            this.lowA | ((byte & 0xf0) << 4),
            false,
            true,
            at,
          )
          await this.field(
            op,
            register,
            index === 4 ? 'horizontal-blanking' : 'vertical-blanking',
            this.lowB | ((byte & 15) << 8),
            false,
            true,
            at,
          )
        }
        return
      }
      if (index === 8 || index === 12) {
        this.lowA = byte
        this.halfAt = where
        return
      }
      if (index === 9 || index === 13) {
        this.lowB = byte
        return
      }
      if (index === 10) {
        this.lowC = byte
        return
      }
      if (index === 11 || index === 14) {
        const first = this.halfAt
        if (first !== undefined) {
          const placed = join(first, where)
          if (index === 11) {
            await this.field(
              op,
              register,
              'horizontal-front-porch',
              this.lowA | ((byte & 192) << 2),
              false,
              true,
              placed,
            )
            await this.field(
              op,
              register,
              'horizontal-sync-width',
              this.lowB | ((byte & 48) << 4),
              false,
              true,
              placed,
            )
            await this.field(
              op,
              register,
              'vertical-front-porch',
              (this.lowC >> 4) | ((byte & 12) << 2),
              false,
              true,
              placed,
            )
            await this.field(
              op,
              register,
              'vertical-sync-width',
              (this.lowC & 15) | ((byte & 3) << 4),
              false,
              true,
              placed,
            )
          } else {
            await this.field(
              op,
              register,
              'horizontal-millimeters',
              this.lowA | ((byte & 240) << 4),
              false,
              true,
              placed,
            )
            await this.field(
              op,
              register,
              'vertical-millimeters',
              this.lowB | ((byte & 15) << 8),
              false,
              true,
              placed,
            )
          }
        }
        this.halfAt = undefined
        return
      }
      if (index === 15) field = 'horizontal-border'
      else if (index === 16) field = 'vertical-border'
      else if (index === 17) {
        await this.field(op, register, 'interlaced', byte >> 7, false, true, where)
        const stereo: u8 = (byte >> 5) & 3
        await this.field(
          op,
          register,
          'stereo-code',
          stereo === 0 ? 0 : (stereo << 1) | (byte & 1),
          false,
          true,
          where,
        )
        await this.field(op, register, 'sync-format', (byte >> 3) & 3, false, true, where)
        await this.field(op, register, 'sync-flags', (byte >> 1) & 3, false, true, where)
        return
      }
    } else {
      if (index === 3 && this.descriptorKnown) {
        field = 'descriptor-tag'
        this.descriptorTag = byte
      }
      if (this.descriptorTag === 0xfd) {
        if (index === 4) {
          this.rangeOffsets = byte
          this.rangeAt = where
          return
        }
        if (index >= 5 && index <= 8) {
          let extra: u16 = 0
          const offsets = this.rangeOffsets
          const revision = this.revision
          if (offsets !== undefined && revision !== undefined && revision >= 4) {
            if (index === 5 && (offsets & 2) !== 0) extra = (offsets & 1) !== 0 ? 255 : 0
            else if (index === 6 && (offsets & 2) !== 0) extra = 255
            else if (index === 7 && (offsets & 8) !== 0) extra = (offsets & 4) !== 0 ? 255 : 0
            else if (index === 8 && (offsets & 8) !== 0) extra = 255
          }
          field =
            index === 5
              ? 'minimum-vertical-hz'
              : index === 6
                ? 'maximum-vertical-hz'
                : index === 7
                  ? 'minimum-horizontal-khz'
                  : 'maximum-horizontal-khz'
          const rangeAt = this.rangeAt
          if (rangeAt !== undefined)
            await this.field(
              op,
              register,
              field,
              (byte as u32) + extra,
              false,
              byte !== 0 && (revision !== undefined || offsets === 0),
              join(rangeAt, where),
            )
          if (index === 8) this.rangeAt = undefined
          return
        }
        if (index === 9) {
          await this.field(
            op,
            register,
            'maximum-pixel-clock-hz',
            (byte as u32) * 10000000,
            false,
            true,
            where,
          )
          return
        }
        if (index === 10) {
          await this.field(
            op,
            register,
            'range-timing-support',
            byte,
            false,
            byte === 0 || byte === 1 || byte === 2 || byte === 4,
            where,
          )
          return
        }
      }
      if (index >= 5 && this.descriptorTag !== 0xfd) {
        field =
          this.descriptorTag === 0xfc
            ? 'monitor-name-byte'
            : this.descriptorTag === 0xff
              ? 'monitor-serial-byte'
              : this.descriptorTag === 0xfe
                ? 'monitor-text-byte'
                : this.descriptorTag === 0xfb
                  ? 'additional-color-point-byte'
                  : this.descriptorTag === 0xfa
                    ? 'additional-standard-timing-byte'
                    : 'descriptor-byte'
      }
    }

    await this.field(op, register, field, byte, false, true, where)
  }
  advance(register: u8, _ack: bool): void {
    this.pointer = register === 255 ? 0 : register + 1
  }
  override async transaction(tx: Transaction<BusItem>): Promise<void> {
    let active = false
    let read = false
    let first = false
    let segmentAccess = false
    let segmentSeen = false
    for (;;) {
      const item = await tx.itemOrEnd()
      if (item === undefined) {
        await this.close('complete', this.consumed())
        this.segment = 0
        return
      }
      const where = this.consumed()
      switch (item.kind) {
        case 'address': {
          await this.close('complete', where)
          segmentAccess =
            !item.tenBit && item.address === this.config.segmentAddress && !item.read && item.ack
          segmentSeen = false
          if (!item.ack) this.segment = 0
          active = !item.tenBit && item.address === this.config.address && item.ack
          read = item.read
          first = !read
          if (!item.tenBit && item.address === this.config.address) {
            const op = this.nextOp
            this.nextOp += 1
            this.currentOp = op
            this.open = true
            await this.out.emit(
              {
                kind: 'begin',
                op,
                tx: tx.id,
                read,
                ack: item.ack,
                segment: this.segment,
              },
              where,
            )
          }
          continue
        }
        case 'restart':
          await this.close('complete', where)
          active = false
          segmentAccess = false
          if (item.droppedBits !== 0) {
            this.pointer = undefined
            this.segment = undefined
          }
          continue
        case 'data': {
          if (segmentAccess) {
            const valid = !segmentSeen && item.byte < 128 && item.ack
            this.segment = valid ? item.byte : undefined
            await this.out.emit(
              {
                kind: 'segment',
                tx: tx.id,
                value: item.byte,
                ack: item.ack,
                valid,
              },
              where,
            )
            segmentSeen = true
            if (!item.ack) {
              this.segment = 0
              segmentAccess = false
            }
            continue
          }
          if (!active || !this.open) {
            if (!item.ack) this.segment = 0
            continue
          }
          const op = this.currentOp
          if (first) {
            first = false
            this.pointer = item.ack ? item.byte : undefined
            await this.out.emit({ kind: 'pointer', op, register: item.byte, ack: item.ack }, where)
            this.checksumAt = undefined
            this.covered = 0
            this.serialSeen = 0
            this.detailed = false
          } else {
            const register = this.pointer
            const segment = this.segment
            let absolute: u32 | undefined = undefined
            if (segment !== undefined && register !== undefined)
              absolute = (segment as u32) * 256 + register
            await this.out.emit(
              {
                kind: 'byte',
                op,
                register,
                absolute,
                value: item.byte,
                read,
                ack: item.ack,
              },
              where,
            )
            if (register !== undefined && (read || item.ack)) {
              await this.interpret(op, register, item.byte, read, where)
              this.advance(register, item.ack)
            }
          }
          if (!item.ack) {
            if (!read) this.pointer = undefined
            await this.close(read ? 'complete' : 'nack', where)
            active = false
            this.segment = 0
          }
          continue
        }
      }
    }
  }
}
export const device = defineBusDecoder({
  name: 'edid',
  category: 'device',
  config: {
    defaults: { address: 80, segmentAddress: 48, initialSegment: 0 },
    validate,
  },
  input: i2cEvents,
  requires: () => ({}),
  output: deviceEvents,
  props: (config) => ({ address: config.address }),
  locales: {
    'en-US': {
      name: 'EDID 1.x',
      summary: 'EDID identity, base display fields, detailed timing and block checksums',
      fields: {
        address: 'I²C address',
        segmentAddress: 'E-DDC segment address',
        initialSegment: 'Assumed initial segment (optional)',
      },
      events: {
        begin: 'Address',
        pointer: 'Register pointer',
        byte: 'Register byte',
        field: 'Decoded field',
        end: 'End',
        timing: 'Established timing',
        segment: 'E-DDC segment',
        'cta-block': 'CTA data block',
        'cta-video': 'CTA short video descriptor',
        'cta-audio': 'CTA short audio descriptor',
      },
      eventFields: {
        segment: 'Segment',
        absolute: 'Absolute byte address',
        block: 'EDID block',
        tag: 'Block tag',
        length: 'Payload length',
        code: 'Video code',
        native: 'Native mode',
        format: 'Audio format',
        channels: 'Audio channels',
        rates: 'Sample rate mask',
        detail: 'Audio detail',
        width: 'Horizontal pixels',
        height: 'Vertical lines',
        refresh: 'Refresh Hz',
        interlaced: 'Interlaced',
        op: 'Operation',
        tx: 'I²C transaction',
        read: 'Read',
        ack: 'Acknowledged',
        register: 'Register',
        value: 'Value',
        field: 'Field',
        negative: 'Negative',
        valid: 'Valid',
        reason: 'Reason',
      },
    },
    'zh-CN': {
      name: 'EDID 1.x',
      summary: 'EDID 标识、基本显示字段、详细时序和块校验和',
      fields: {
        address: 'I²C地址',
        segmentAddress: 'E-DDC段地址',
        initialSegment: '假定起始段（可选）',
      },
      events: {
        begin: '地址',
        pointer: '寄存器指针',
        byte: '寄存器字节',
        field: '解码字段',
        end: '结束',
        timing: '已定义时序',
        segment: 'E-DDC段',
        'cta-block': 'CTA数据块',
        'cta-video': 'CTA短视频描述符',
        'cta-audio': 'CTA短音频描述符',
      },
      eventFields: {
        op: '操作号',
        tx: 'I²C 事务',
        read: '读取',
        ack: '应答',
        register: '寄存器',
        value: '值',
        field: '字段',
        negative: '负数',
        valid: '有效',
        reason: '原因',
      },
    },
  },
  partial: 'drain',
  recovery: {
    incomplete: 'incomplete',
    truncated: 'aborted',
    missing: 'aborted',
    gap: 'aborted',
    end: 'aborted',
  },
  decoder: Device,
})
