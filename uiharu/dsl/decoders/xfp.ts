// SPDX-License-Identifier: Apache-2.0

/** XFP management. Implemented from XFP MSA Rev4.5 §§4–5 (SNIA public standard). Original implementation from primary tables; pinned reference consulted only for scope inventory.
 * Register pointers persist across STOP, but unknown pointers never become fabricated zeroes.
 * Bytes stream immediately; at most one unfinished two-byte semantic value is retained.
 */
import type { ItemOf, Provenance, Tick, Transaction, bool, u16, u32, u64, u8 } from 'uiharu'
import { BusDecoder, defineBusDecoder, eventSchema, join } from 'uiharu'
import { i2cEvents } from '../contracts/i2c'

type BusItem = ItemOf<typeof i2cEvents>
export type OverlayEnd = 'complete' | 'incomplete' | 'aborted' | 'nack'
export type Field =
  | 'table-select'
  | 'temperature-256ths-celsius'
  | 'tx-bias-2microamp'
  | 'tx-power-tenth-microwatt'
  | 'rx-power-tenth-microwatt'
  | 'auxiliary-raw'
  | 'identifier'
  | 'alarm-flags'
  | 'control-status'
  | 'vendor-name-byte'
  | 'vendor-oui-byte'
  | 'part-number-byte'
  | 'serial-number-byte'
  | 'date-code-byte'
  | 'unknown-table'
  | 'signal-data-rate-megabits'
  | 'lineside-loopback'
  | 'xfi-loopback'
  | 'synchronous-refclk'
  | 'vps-lowest-millivolts'
  | 'vps-supply-millivolts'
  | 'vps-bypassed-millivolts'
  | 'vps-bypass-enabled'
  | 'ber-byte'
  | 'wavelength-control-byte'
  | 'fec-control-byte'
  | 'interrupt-mask'
  | 'serial-error-checking'
  | 'power-class'
  | 'cdr-present'
  | 'refclk-required'
  | 'clei-present'
  | 'connector-code'
  | 'transceiver-byte'
  | 'encoding-mask'
  | 'minimum-bitrate-megabits'
  | 'maximum-bitrate-megabits'
  | 'length-smf-meters'
  | 'length-extended-50um-meters'
  | 'length-50um-meters'
  | 'length-625um-meters'
  | 'length-copper-meters'
  | 'transmitter-technology-code'
  | 'wavelength-controlled'
  | 'cooled-transmitter'
  | 'apd-detector'
  | 'tunable-transmitter'
  | 'cdr-support-mask'
  | 'vendor-revision-byte'
  | 'wavelength-picometers'
  | 'wavelength-tolerance-picometers'
  | 'maximum-case-celsius'
  | 'maximum-power-milliwatts'
  | 'power-down-milliwatts'
  | 'current-5v-milliamps'
  | 'current-33v-milliamps'
  | 'current-18v-milliamps'
  | 'current-negative-5v-milliamps'
  | 'manufacture-year'
  | 'manufacture-month'
  | 'manufacture-day'
  | 'diagnostics-ber-supported'
  | 'rx-power-average'
  | 'enhanced-options-mask'
  | 'auxiliary-1-type'
  | 'auxiliary-2-type'
  | 'vendor-specific-byte'
  | 'register'
export type Capability =
  | '10gbase-sr'
  | '10gbase-lr'
  | '10gbase-er'
  | '10gbase-lrm'
  | '10gbase-sw'
  | '10gbase-lw'
  | '10gbase-ew'
  | '1200-mx-sn-i'
  | '1200-sm-ll-l'
  | 'extended-reach-1550nm'
  | 'intermediate-reach-1300nm-fp'
  | '1000base-sx-1xfc-mmf'
  | '1000base-lx-1xfc-smf'
  | '2xfc-mmf'
  | '2xfc-smf'
  | 'oc48-sr'
  | 'oc48-ir'
  | 'oc48-lr'
  | 'i64-1r'
  | 'i64-1'
  | 'i64-2r'
  | 'i64-2'
  | 'i64-3'
  | 'i64-5'
  | 's64-1'
  | 's64-2a'
  | 's64-2b'
  | 's64-3a'
  | 's64-3b'
  | 's64-5a'
  | 's64-5b'
  | 'l64-1'
  | 'l64-2a'
  | 'l64-2b'
  | 'l64-2c'
  | 'l64-3'
  | 'g959-1-p1l1-2d2'
  | 'v64-2a'
  | 'v64-2b'
  | 'v64-3'
  | 'encoding-64b66b'
  | 'encoding-8b10b'
  | 'encoding-sonet'
  | 'encoding-nrz'
  | 'encoding-rz'
  | 'cdr-9950'
  | 'cdr-10300'
  | 'cdr-10500'
  | 'cdr-10700'
  | 'cdr-11100'
  | 'lineside-loopback-supported'
  | 'xfi-loopback-supported'
  | 'optional-vps'
  | 'soft-tx-disable'
  | 'soft-power-down'
  | 'vps-regulator'
  | 'vps-bypassed-regulator'
  | 'active-fec'
  | 'wavelength-tunable'
  | 'cmu-refclk'
export type ThresholdChannel =
  | 'temperature-256ths-celsius'
  | 'tx-bias-2microamp'
  | 'tx-power-tenth-microwatt'
  | 'rx-power-tenth-microwatt'
  | 'auxiliary-1-raw'
  | 'auxiliary-2-raw'
export type ThresholdLevel = 'high-alarm' | 'low-alarm' | 'high-warning' | 'low-warning'
export type AuxiliaryType =
  | 'not-implemented'
  | 'apd-bias-voltage'
  | 'tec-current'
  | 'laser-temperature'
  | 'laser-wavelength'
  | 'supply-5v-voltage'
  | 'supply-33v-voltage'
  | 'supply-18v-voltage'
  | 'supply-negative-5v-voltage'
  | 'supply-5v-current'
  | 'supply-33v-current'
  | 'supply-18v-current'
  | 'supply-negative-5v-current'
  | 'reserved'
export type Medium = 'single-mode' | 'extended-50um' | '50um' | '625um' | 'copper'
export type Description =
  | 'unknown-or-reserved'
  | 'vendor-specific'
  | 'sc'
  | 'fc-style1-copper'
  | 'fc-style2-copper'
  | 'bnc-tnc'
  | 'fc-coaxial'
  | 'fiberjack'
  | 'lc'
  | 'mt-rj'
  | 'mu'
  | 'sg'
  | 'optical-pigtail'
  | 'hssdc-ii'
  | 'copper-pigtail'
  | '850nm-vcsel'
  | '1310nm-vcsel'
  | '1550nm-vcsel'
  | '1310nm-fp'
  | '1310nm-dfb'
  | '1550nm-dfb'
  | '1310nm-eml'
  | '1550nm-eml'
  | 'copper-or-other'
export type DeviceEvent =
  | { kind: 'begin'; op: u64; tx: u64; read: bool; ack: bool }
  | { kind: 'pointer'; op: u64; register: u8; ack: bool }
  | { kind: 'byte'; op: u64; register: u8 | undefined; value: u8; read: bool; ack: bool }
  | { kind: 'field'; op: u64; register: u8; field: Field; value: u32; negative: bool; valid: bool }
  | { kind: 'auxiliary-type'; op: u64; channel: u8; code: u8; type: AuxiliaryType }
  | {
      kind: 'link'
      op: u64
      register: u8
      medium: Medium
      meters: u32 | undefined
      greaterThan: bool
      specified: bool
    }
  | { kind: 'description'; op: u64; register: u8; code: u8; description: Description }
  | {
      kind: 'threshold'
      op: u64
      register: u8
      channel: ThresholdChannel
      level: ThresholdLevel
      value: u32
      negative: bool
    }
  | { kind: 'capability'; op: u64; register: u8; name: Capability; enabled: bool }
  | { kind: 'end'; op: u64; reason: OverlayEnd }
export const deviceEvents = eventSchema<DeviceEvent>('xfp.events', 1).roles({
  begin: { role: 'address', errorWhen: { ack: false } },
  pointer: 'address',
  byte: 'data',
  field: { role: 'data', errorWhen: { valid: false } },
  end: 'framing',
  threshold: 'data',
  capability: 'control',
  link: 'data',
  description: 'data',
  'auxiliary-type': 'control',
})
export interface DeviceConfig {
  address: u16
  initialTable: u8
}
function validate(config: DeviceConfig): string | undefined {
  if (config.address > 127) return 'decode.config.address'
  return undefined
}

function auxiliaryType(code: u8): AuxiliaryType {
  if (code === 0) return 'not-implemented'
  if (code === 1) return 'apd-bias-voltage'
  if (code === 3) return 'tec-current'
  if (code === 4) return 'laser-temperature'
  if (code === 5) return 'laser-wavelength'
  if (code === 6) return 'supply-5v-voltage'
  if (code === 7) return 'supply-33v-voltage'
  if (code === 8) return 'supply-18v-voltage'
  if (code === 9) return 'supply-negative-5v-voltage'
  if (code === 10) return 'supply-5v-current'
  if (code === 13) return 'supply-33v-current'
  if (code === 14) return 'supply-18v-current'
  if (code === 15) return 'supply-negative-5v-current'
  return 'reserved'
}
function description(code: u8, connector: bool): Description {
  if (!connector) {
    if (code === 0) return '850nm-vcsel'
    if (code === 1) return '1310nm-vcsel'
    if (code === 2) return '1550nm-vcsel'
    if (code === 3) return '1310nm-fp'
    if (code === 4) return '1310nm-dfb'
    if (code === 5) return '1550nm-dfb'
    if (code === 6) return '1310nm-eml'
    if (code === 7) return '1550nm-eml'
    if (code === 8) return 'copper-or-other'
    return 'unknown-or-reserved'
  }
  if (code === 1) return 'sc'
  if (code === 2) return 'fc-style1-copper'
  if (code === 3) return 'fc-style2-copper'
  if (code === 4) return 'bnc-tnc'
  if (code === 5) return 'fc-coaxial'
  if (code === 6) return 'fiberjack'
  if (code === 7) return 'lc'
  if (code === 8) return 'mt-rj'
  if (code === 9) return 'mu'
  if (code === 10) return 'sg'
  if (code === 11) return 'optical-pigtail'
  if (code === 32) return 'hssdc-ii'
  if (code === 33) return 'copper-pigtail'
  if (code >= 128) return 'vendor-specific'
  return 'unknown-or-reserved'
}
function capability(register: u8, bit: u8): Capability | undefined {
  if (register === 131) {
    if (bit === 7) return '10gbase-sr'
    if (bit === 6) return '10gbase-lr'
    if (bit === 5) return '10gbase-er'
    if (bit === 4) return '10gbase-lrm'
    if (bit === 3) return '10gbase-sw'
    if (bit === 2) return '10gbase-lw'
    if (bit === 1) return '10gbase-ew'
  }
  if (register === 132) {
    if (bit === 7) return '1200-mx-sn-i'
    if (bit === 6) return '1200-sm-ll-l'
    if (bit === 5) return 'extended-reach-1550nm'
    if (bit === 4) return 'intermediate-reach-1300nm-fp'
  }
  if (register === 134) {
    if (bit === 7) return '1000base-sx-1xfc-mmf'
    if (bit === 6) return '1000base-lx-1xfc-smf'
    if (bit === 5) return '2xfc-mmf'
    if (bit === 4) return '2xfc-smf'
    if (bit === 3) return 'oc48-sr'
    if (bit === 2) return 'oc48-ir'
    if (bit === 1) return 'oc48-lr'
  }
  if (register === 135) {
    if (bit === 7) return 'i64-1r'
    if (bit === 6) return 'i64-1'
    if (bit === 5) return 'i64-2r'
    if (bit === 4) return 'i64-2'
    if (bit === 3) return 'i64-3'
    if (bit === 2) return 'i64-5'
  }
  if (register === 136) {
    if (bit === 7) return 's64-1'
    if (bit === 6) return 's64-2a'
    if (bit === 5) return 's64-2b'
    if (bit === 4) return 's64-3a'
    if (bit === 3) return 's64-3b'
    if (bit === 2) return 's64-5a'
    if (bit === 1) return 's64-5b'
  }
  if (register === 137) {
    if (bit === 7) return 'l64-1'
    if (bit === 6) return 'l64-2a'
    if (bit === 5) return 'l64-2b'
    if (bit === 4) return 'l64-2c'
    if (bit === 3) return 'l64-3'
    if (bit === 2) return 'g959-1-p1l1-2d2'
  }
  if (register === 138) {
    if (bit === 7) return 'v64-2a'
    if (bit === 6) return 'v64-2b'
    if (bit === 5) return 'v64-3'
  }
  if (register === 139) {
    if (bit === 7) return 'encoding-64b66b'
    if (bit === 6) return 'encoding-8b10b'
    if (bit === 5) return 'encoding-sonet'
    if (bit === 4) return 'encoding-nrz'
    if (bit === 3) return 'encoding-rz'
  }
  if (register === 164) {
    if (bit === 7) return 'cdr-9950'
    if (bit === 6) return 'cdr-10300'
    if (bit === 5) return 'cdr-10500'
    if (bit === 4) return 'cdr-10700'
    if (bit === 3) return 'cdr-11100'
    if (bit === 1) return 'lineside-loopback-supported'
    if (bit === 0) return 'xfi-loopback-supported'
  }
  if (register === 221) {
    if (bit === 7) return 'optional-vps'
    if (bit === 6) return 'soft-tx-disable'
    if (bit === 5) return 'soft-power-down'
    if (bit === 4) return 'vps-regulator'
    if (bit === 3) return 'vps-bypassed-regulator'
    if (bit === 2) return 'active-fec'
    if (bit === 1) return 'wavelength-tunable'
    if (bit === 0) return 'cmu-refclk'
  }
  return undefined
}
class Device extends BusDecoder<DeviceConfig, BusItem, DeviceEvent, OverlayEnd> {
  pointer: u8 | undefined = undefined
  nextOp: u64 = 1
  open = false
  currentOp: u64 = 0
  half: u8 | undefined = undefined
  halfAt: Provenance | undefined = undefined
  table: u8 = this.config.initialTable
  tableKnown = true
  override hold(): Tick | undefined {
    return this.halfAt?.span.start
  }
  override onGap(): void {
    this.pointer = undefined
    this.half = undefined
    this.halfAt = undefined
    this.tableKnown = false
  }
  async close(reason: OverlayEnd, where: Provenance): Promise<void> {
    let ending = reason
    if (this.open) {
      const op = this.currentOp
      if (ending === 'complete' && this.half !== undefined) ending = 'incomplete'
      this.half = undefined
      this.halfAt = undefined
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
    await this.out.emit({ kind: 'field', op, register, field, value, negative, valid }, where)
  }
  async interpret(op: u64, register: u8, byte: u8, read: bool, where: Provenance): Promise<void> {
    if (register === 127) {
      this.table = byte
      this.tableKnown = read || byte === 1
      await this.field(op, register, 'table-select', byte, false, true, where)
      return
    }
    if (
      ((register >= 131 && register <= 139) || register === 164 || register === 221) &&
      this.tableKnown &&
      this.table === 1
    ) {
      for (let bit = 0; bit < 8; bit += 1) {
        // eslint-disable-next-line @typescript-eslint/no-unnecessary-type-assertion -- The DSL loop index must narrow explicitly to this u8 argument.
        const name = capability(register, bit as u8)
        if (name !== undefined)
          await this.out.emit(
            { kind: 'capability', op, register, name, enabled: (byte & (1 << (bit as u64))) !== 0 },
            where,
          )
      }
    }
    const threshold = register >= 2 && register <= 57 && !(register >= 10 && register <= 17)
    const upperPair =
      this.tableKnown &&
      this.table === 1 &&
      ((register >= 186 && register <= 189) || (register >= 212 && register <= 217))
    if (threshold || upperPair) {
      if ((register & 1) === 0) {
        this.half = byte
        this.halfAt = where
        return
      }
      const first = this.half
      const firstAt = this.halfAt
      if (first === undefined) return
      if (firstAt === undefined) return
      const raw: u16 = ((first as u16) << 8) | byte
      const placed = join(firstAt, where)
      this.half = undefined
      this.halfAt = undefined
      if (threshold) {
        let channel: ThresholdChannel = 'temperature-256ths-celsius'
        if (register >= 51) channel = 'auxiliary-2-raw'
        else if (register >= 43) channel = 'auxiliary-1-raw'
        else if (register >= 35) channel = 'rx-power-tenth-microwatt'
        else if (register >= 27) channel = 'tx-power-tenth-microwatt'
        else if (register >= 19) channel = 'tx-bias-2microamp'
        const position: u8 = ((register - 3) % 8) / 2
        const level: ThresholdLevel =
          position === 0
            ? 'high-alarm'
            : position === 1
              ? 'low-alarm'
              : position === 2
                ? 'high-warning'
                : 'low-warning'
        const negative = register < 10 && (raw & 0x8000) !== 0
        await this.out.emit(
          {
            kind: 'threshold',
            op,
            register: register - 1,
            channel,
            level,
            value: negative ? 65536 - raw : raw,
            negative,
          },
          placed,
        )
      } else if (register <= 189)
        await this.field(
          op,
          register - 1,
          register === 187 ? 'wavelength-picometers' : 'wavelength-tolerance-picometers',
          (raw as u32) * (register === 187 ? 50 : 5),
          false,
          true,
          placed,
        )
      else {
        const valid = first >= 48 && first <= 57 && byte >= 48 && byte <= 57
        let value: u32 = valid ? ((first - 48) as u32) * 10 + (byte - 48) : 0
        const field: Field =
          register === 213
            ? 'manufacture-year'
            : register === 215
              ? 'manufacture-month'
              : 'manufacture-day'
        let range = true
        if (register === 213) value += 2000
        else range = value >= 1 && value <= (register === 215 ? 12 : 31)
        await this.field(op, register - 1, field, value, false, valid && range, placed)
      }
      return
    }
    if (register >= 96 && register <= 109 && register !== 98 && register !== 99 && read) {
      if ((register & 1) === 0) {
        this.half = byte
        this.halfAt = where
        return
      }
      if (this.half === undefined) return
      const raw: u16 = (this.half << 8) | byte
      const at = join(this.halfAt!, where)
      this.half = undefined
      this.halfAt = undefined
      let field: Field = 'auxiliary-raw'
      if (register === 97) field = 'temperature-256ths-celsius'
      if (register === 101) field = 'tx-bias-2microamp'
      if (register === 103) field = 'tx-power-tenth-microwatt'
      if (register === 105) field = 'rx-power-tenth-microwatt'
      const negative = register === 97 && (raw & 0x8000) !== 0
      await this.field(op, register - 1, field, negative ? 65536 - raw : raw, negative, true, at)
      return
    }
    if (register === 1) {
      await this.field(
        op,
        register,
        'signal-data-rate-megabits',
        9500 + (byte >> 4) * 200,
        false,
        true,
        where,
      )
      await this.field(op, register, 'lineside-loopback', (byte >> 2) & 1, false, true, where)
      await this.field(op, register, 'xfi-loopback', (byte >> 1) & 1, false, true, where)
      await this.field(op, register, 'synchronous-refclk', byte & 1, false, true, where)
      return
    }
    if (register === 58) {
      await this.field(
        op,
        register,
        'vps-lowest-millivolts',
        1800 - (byte >> 4) * 100,
        false,
        true,
        where,
      )
      await this.field(
        op,
        register,
        'vps-supply-millivolts',
        1800 - (byte & 15) * 100,
        false,
        true,
        where,
      )
      return
    }
    if (register === 59) {
      await this.field(
        op,
        register,
        'vps-bypassed-millivolts',
        1800 - (byte >> 4) * 100,
        false,
        true,
        where,
      )
      await this.field(op, register, 'vps-bypass-enabled', byte & 1, false, true, where)
      return
    }
    if (register >= 128 && this.tableKnown && this.table === 1) {
      if (register === 129) {
        await this.field(op, register, 'power-class', (byte >> 6) + 1, false, true, where)
        await this.field(op, register, 'cdr-present', (byte & 32) === 0 ? 1 : 0, false, true, where)
        await this.field(
          op,
          register,
          'refclk-required',
          (byte & 16) === 0 ? 1 : 0,
          false,
          true,
          where,
        )
        await this.field(op, register, 'clei-present', (byte >> 3) & 1, false, true, where)
        return
      }
      if (register === 130)
        await this.out.emit(
          { kind: 'description', op, register, code: byte, description: description(byte, true) },
          where,
        )
      if (register === 140 || register === 141) {
        await this.field(
          op,
          register,
          register === 140 ? 'minimum-bitrate-megabits' : 'maximum-bitrate-megabits',
          (byte as u32) * 100,
          false,
          byte !== 0,
          where,
        )
        return
      }
      if (register >= 142 && register <= 146) {
        const field: Field =
          register === 142
            ? 'length-smf-meters'
            : register === 143
              ? 'length-extended-50um-meters'
              : register === 144
                ? 'length-50um-meters'
                : register === 145
                  ? 'length-625um-meters'
                  : 'length-copper-meters'
        const scale: u32 = register === 142 ? 1000 : register === 143 ? 2 : 1
        const medium: Medium =
          register === 142
            ? 'single-mode'
            : register === 143
              ? 'extended-50um'
              : register === 144
                ? '50um'
                : register === 145
                  ? '625um'
                  : 'copper'
        let meters: u32 | undefined = undefined
        if (byte !== 0) meters = ((byte === 255 ? 254 : byte) as u32) * scale
        await this.out.emit(
          {
            kind: 'link',
            op,
            register,
            medium,
            meters,
            greaterThan: byte === 255,
            specified: byte !== 0,
          },
          where,
        )
        if (byte !== 0 && byte !== 255)
          await this.field(op, register, field, (byte as u32) * scale, false, true, where)
        return
      }
      if (register === 147) {
        await this.out.emit(
          {
            kind: 'description',
            op,
            register,
            code: byte >> 4,
            description: description(byte >> 4, false),
          },
          where,
        )
        await this.field(
          op,
          register,
          'transmitter-technology-code',
          byte >> 4,
          false,
          byte >> 4 <= 8,
          where,
        )
        await this.field(op, register, 'wavelength-controlled', (byte >> 3) & 1, false, true, where)
        await this.field(op, register, 'cooled-transmitter', (byte >> 2) & 1, false, true, where)
        await this.field(op, register, 'apd-detector', (byte >> 1) & 1, false, true, where)
        await this.field(op, register, 'tunable-transmitter', byte & 1, false, true, where)
        return
      }
      if (register === 192 || register === 193) {
        await this.field(
          op,
          register,
          register === 192 ? 'maximum-power-milliwatts' : 'power-down-milliwatts',
          (byte as u32) * (register === 192 ? 20 : 10),
          false,
          true,
          where,
        )
        return
      }
      if (register === 194) {
        await this.field(op, register, 'current-5v-milliamps', (byte >> 4) * 50, false, true, where)
        await this.field(
          op,
          register,
          'current-33v-milliamps',
          (byte & 15) * 100,
          false,
          true,
          where,
        )
        return
      }
      if (register === 195) {
        await this.field(
          op,
          register,
          'current-18v-milliamps',
          (byte >> 4) * 100,
          false,
          true,
          where,
        )
        await this.field(
          op,
          register,
          'current-negative-5v-milliamps',
          (byte & 15) * 50,
          false,
          true,
          where,
        )
        return
      }
      if (register === 220) {
        await this.field(
          op,
          register,
          'diagnostics-ber-supported',
          (byte >> 4) & 1,
          false,
          true,
          where,
        )
        await this.field(op, register, 'rx-power-average', (byte >> 3) & 1, false, true, where)
        return
      }
      if (register === 222) {
        await this.out.emit(
          {
            kind: 'auxiliary-type',
            op,
            channel: 1,
            code: byte >> 4,
            type: auxiliaryType(byte >> 4),
          },
          where,
        )
        await this.out.emit(
          {
            kind: 'auxiliary-type',
            op,
            channel: 2,
            code: byte & 15,
            type: auxiliaryType(byte & 15),
          },
          where,
        )
        await this.field(op, register, 'auxiliary-1-type', byte >> 4, false, true, where)
        await this.field(op, register, 'auxiliary-2-type', byte & 15, false, true, where)
        return
      }
    }
    let field: Field = 'register'
    if (register === 0 || (register === 128 && this.tableKnown && this.table === 1))
      field = 'identifier'
    if (register >= 80 && register <= 87) field = 'alarm-flags'
    if (register >= 88 && register <= 95) field = 'interrupt-mask'
    if (register === 70 || register === 71) field = 'ber-byte'
    if (register >= 72 && register <= 75) field = 'wavelength-control-byte'
    if (register >= 76 && register <= 79) field = 'fec-control-byte'
    if (register === 118) field = 'serial-error-checking'
    if (register >= 110 && register <= 111) field = 'control-status'
    if (register >= 128) {
      if (!this.tableKnown) field = 'unknown-table'
      else if (this.table === 1) {
        if (register === 130) field = 'connector-code'
        if (register >= 131 && register <= 138) field = 'transceiver-byte'
        if (register === 139) field = 'encoding-mask'
        if (register === 164) field = 'cdr-support-mask'
        if (register === 184 || register === 185) field = 'vendor-revision-byte'
        if (register === 190) field = 'maximum-case-celsius'
        if (register === 221) field = 'enhanced-options-mask'
        if (register >= 224) field = 'vendor-specific-byte'
        if (register >= 148 && register <= 163) field = 'vendor-name-byte'
        if (register >= 165 && register <= 167) field = 'vendor-oui-byte'
        if (register >= 168 && register <= 183) field = 'part-number-byte'
        if (register >= 196 && register <= 211) field = 'serial-number-byte'
        if (register >= 212 && register <= 219) field = 'date-code-byte'
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
    for (;;) {
      const item = await tx.itemOrEnd()
      if (item === undefined) {
        await this.close('complete', this.consumed())
        return
      }
      const where = this.consumed()
      switch (item.kind) {
        case 'address': {
          await this.close('complete', where)
          active = !item.tenBit && item.address === this.config.address && item.ack
          read = item.read
          first = !read
          if (!item.tenBit && item.address === this.config.address) {
            const op = this.nextOp
            this.nextOp += 1
            this.currentOp = op
            this.open = true
            await this.out.emit({ kind: 'begin', op, tx: tx.id, read, ack: item.ack }, where)
          }
          continue
        }
        case 'restart':
          await this.close('complete', where)
          active = false
          if (item.droppedBits !== 0) this.pointer = undefined
          continue
        case 'data': {
          if (!active || !this.open) continue
          const op = this.currentOp
          if (first) {
            first = false
            this.pointer = item.ack ? item.byte : undefined
            await this.out.emit({ kind: 'pointer', op, register: item.byte, ack: item.ack }, where)
          } else {
            const register = this.pointer
            await this.out.emit(
              { kind: 'byte', op, register, value: item.byte, read, ack: item.ack },
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
          }
          continue
        }
      }
    }
  }
}
export const device = defineBusDecoder({
  name: 'xfp',
  category: 'device',
  config: { defaults: { address: 80, initialTable: 1 }, validate },
  input: i2cEvents,
  requires: () => ({}),
  output: deviceEvents,
  props: (config) => ({ address: config.address }),
  locales: {
    'en-US': {
      name: 'XFP management',
      summary: 'Thresholds, control, diagnostics and page-one transceiver/link/identity fields',
      fields: { address: 'I²C address', initialTable: 'Initial upper memory table' },
      events: {
        begin: 'Address',
        pointer: 'Register pointer',
        byte: 'Register byte',
        field: 'Decoded field',
        threshold: 'Alarm/warning threshold',
        capability: 'Module capability',
        link: 'Supported link length',
        description: 'Physical interface',
        'auxiliary-type': 'Auxiliary monitor type',
        end: 'End',
      },
      eventFields: {
        channel: 'Threshold channel',
        level: 'Threshold level',
        name: 'Capability',
        enabled: 'Enabled',
        medium: 'Medium',
        meters: 'Meters',
        greaterThan: 'Strict lower bound',
        specified: 'Length specified',
        code: 'Code',
        description: 'Description',
        type: 'Monitor type',
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
      name: 'XFP management',
      summary: '诊断测量、内存表选择和序列标识',
      fields: { address: 'I²C 地址', initialTable: '初始上部内存表' },
      events: {
        begin: '地址',
        pointer: '寄存器指针',
        byte: '寄存器字节',
        field: '解码字段',
        threshold: '告警/警告阈值',
        capability: '模块能力',
        link: '支持的链路长度',
        description: '物理接口',
        'auxiliary-type': '辅助监测类型',
        end: '结束',
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
