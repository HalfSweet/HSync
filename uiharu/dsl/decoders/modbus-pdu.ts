// SPDX-License-Identifier: Apache-2.0

/**
 * Modbus application fields from the public MODBUS Application Protocol Specification
 * V1.1b3 (2012-04-26), sections 6.1–6.21 and 7:
 * https://modbus.org/docs/Modbus_Application_Protocol_V1_1b3.pdf
 *
 * Direction is configured: a single RTU stream does not identify the transmitter.
 * Fields remain observations until the transport end validates CRC and completeness.
 * No frame buffering: one two-byte word and fixed scalar counters are retained.
 */
import type { Provenance, Range, Tick, bool, u16, u32, u64, u8 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, over } from 'uiharu'
import { modbusEvents, type EndReason, type ModbusEvent } from '../contracts/modbus'

export type Direction = 'request' | 'response'
export interface ModbusPduConfig {
  direction: Direction
}
export type FieldName =
  | 'address'
  | 'quantity'
  | 'value'
  | 'read-address'
  | 'read-quantity'
  | 'write-address'
  | 'write-quantity'
  | 'and-mask'
  | 'or-mask'
  | 'subfunction'
  | 'status'
  | 'event-count'
  | 'message-count'
  | 'exception'
  | 'byte-count'
  | 'server-id'
  | 'run-indicator'
  | 'reference-type'
  | 'file-number'
  | 'record-number'
  | 'record-length'
  | 'subresponse-length'
  | 'fifo-count'
  | 'mei-type'
  | 'read-device-id'
  | 'conformity'
  | 'more-follows'
  | 'next-object-id'
  | 'object-count'
  | 'object-id'
  | 'object-length'
  | 'event'
export type ModbusPduEvent =
  | {
      kind: 'begin'
      frame: u64
      address: u8
      function: u8 | undefined
      direction: Direction
      partial: bool
    }
  | { kind: 'field'; frame: u64; field: FieldName; value: u32 }
  | { kind: 'register'; frame: u64; index: u32; value: u16 }
  | { kind: 'bits'; frame: u64; offset: u32; value: u8 }
  | { kind: 'data'; frame: u64; offset: u32; value: u8 }
  | { kind: 'end'; frame: u64; reason: EndReason; lengthOk: bool; supported: bool; valid: bool }
export const modbusPduEvents = eventSchema<ModbusPduEvent>('modbus-pdu.events', 1).roles({
  begin: 'command',
  field: 'control',
  register: 'data',
  bits: 'data',
  data: 'data',
  end: {
    role: 'framing',
    errorWhen: [
      { lengthOk: false },
      { reason: 'bad-crc' },
      { reason: 'short' },
      { reason: 'broken' },
      { reason: 'aborted' },
    ],
  },
})

function supported(functionCode: u8): bool {
  switch (functionCode) {
    case 1:
    case 2:
    case 3:
    case 4:
    case 5:
    case 6:
    case 7:
    case 8:
    case 11:
    case 12:
    case 15:
    case 16:
    case 17:
    case 20:
    case 21:
    case 24:
    case 43:
    case 22:
    case 23:
      return true
    default:
      return false
  }
}

/** Name of a two-byte field at the given payload offset. */
function wordField(code: u8, response: bool, offset: u32): FieldName | undefined {
  if (code === 24) {
    if (offset === 0) return response ? 'byte-count' : 'address'
    if (response && offset === 2) return 'fifo-count'
  } else if (code === 5 || code === 6) {
    if (offset === 0) return 'address'
    if (offset === 2) return 'value'
  } else if (code === 22) {
    if (offset === 0) return 'address'
    if (offset === 2) return 'and-mask'
    if (offset === 4) return 'or-mask'
  } else if (code === 8) {
    if (offset === 0) return 'subfunction'
    if (offset === 2) return 'value'
  } else if (response) {
    if (code === 11) {
      if (offset === 0) return 'status'
      if (offset === 2) return 'event-count'
    } else if (code === 12) {
      if (offset === 1) return 'status'
      if (offset === 3) return 'event-count'
      if (offset === 5) return 'message-count'
    } else if (code === 15 || code === 16) {
      if (offset === 0) return 'address'
      if (offset === 2) return 'quantity'
    }
  } else {
    if (code === 1 || code === 2 || code === 3 || code === 4 || code === 15 || code === 16) {
      if (offset === 0) return 'address'
      if (offset === 2) return 'quantity'
    } else if (code === 23) {
      if (offset === 0) return 'read-address'
      if (offset === 2) return 'read-quantity'
      if (offset === 4) return 'write-address'
      if (offset === 6) return 'write-quantity'
    }
  }
  return undefined
}

function byteCountOffset(code: u8, response: bool): u32 | undefined {
  if (response) {
    if (
      code === 1 ||
      code === 2 ||
      code === 3 ||
      code === 4 ||
      code === 12 ||
      code === 17 ||
      code === 23 ||
      code === 20 ||
      code === 21
    )
      return 0
  } else {
    if (code === 20 || code === 21) return 0
    if (code === 15 || code === 16) return 4
    if (code === 23) return 8
  }
  return undefined
}

class ModbusPdu extends StreamDecoder<ModbusPduConfig, ModbusEvent, ModbusPduEvent> {
  frame: u64 | undefined = undefined
  code: u8 = 0
  exception = false
  partial = false
  known = false
  malformed = false
  count: u32 = 0
  advertised: u8 | undefined = undefined
  quantity: u16 | undefined = undefined
  readQuantity: u16 | undefined = undefined
  writeQuantity: u16 | undefined = undefined
  pending: u8 | undefined = undefined
  pendingAt: Provenance | undefined = undefined
  pendingField: FieldName | undefined = undefined
  registerIndex: u32 = 0
  crcOk = false
  subOffset: u32 = 0
  remaining: u32 = 0
  recordLength: u16 = 0
  fifoBytes: u16 = 0
  fifoCount: u16 = 0
  mei: u8 = 0
  objectCount: u8 = 0
  objects: u32 = 0

  override hold(): Tick | undefined {
    return this.pendingAt?.span.start
  }

  clear(): void {
    this.frame = undefined
    this.pending = undefined
    this.pendingAt = undefined
    this.pendingField = undefined
  }

  override async onGap(range: Range): Promise<void> {
    await this.cut(range.start)
  }
  override async onEnd(tick: Tick): Promise<void> {
    await this.cut(tick)
  }
  async cut(tick: Tick): Promise<void> {
    if (this.frame !== undefined) {
      await this.out.emit(
        {
          kind: 'end',
          frame: this.frame,
          reason: 'aborted',
          lengthOk: false,
          supported: this.known,
          valid: false,
        },
        over(tick, tick, tick),
      )
    }
    this.clear()
  }

  async field(field: FieldName, value: u32, at: Provenance): Promise<void> {
    if (this.frame !== undefined)
      await this.out.emit({ kind: 'field', frame: this.frame, field, value }, at)
  }

  /** Retains exactly one high byte, including its provenance if a lower batch ends here. */
  async word(byte: u8, at: Provenance, field: FieldName | undefined): Promise<void> {
    if (this.pending === undefined) {
      this.pending = byte
      this.pendingAt = at
      this.pendingField = field
      return
    }
    const value: u16 = (this.pending << 8) | byte
    const first = this.pendingAt
    const named = this.pendingField
    this.pending = undefined
    this.pendingAt = undefined
    this.pendingField = undefined
    if (first === undefined) return
    const placed = over(first.span.start, at.span.end, at.settled)
    if (named !== undefined) {
      if (named === 'file-number' && value === 0) this.malformed = true
      if (named === 'record-number' && value > 9999) this.malformed = true
      if (named === 'record-length') {
        this.recordLength = value
        if (value < 1 || value > 125) this.malformed = true
      }
      if (named === 'fifo-count') this.fifoCount = value
      if (named === 'byte-count' && this.code === 24) this.fifoBytes = value
      if (named === 'subfunction' && value > 0x14) this.malformed = true
      if (named === 'quantity') this.quantity = value
      if (named === 'read-quantity') this.readQuantity = value
      if (named === 'write-quantity') this.writeQuantity = value
      if (named === 'value' && this.code === 5 && value !== 0 && value !== 0xff00)
        this.malformed = true
      await this.field(named, value, placed)
    } else if (this.frame !== undefined) {
      await this.out.emit(
        { kind: 'register', frame: this.frame, index: this.registerIndex, value },
        placed,
      )
      this.registerIndex += 1
    }
  }

  async byte(byte: u8, at: Provenance): Promise<void> {
    const offset = this.count
    if (this.count < 0xffffffff) this.count += 1
    if (this.count > 252) this.malformed = true
    if (this.frame === undefined) return
    const response = this.config.direction === 'response'
    if (this.exception) {
      if (offset === 0) await this.field('exception', byte, at)
      else this.malformed = true
      return
    }
    if (this.code === 20 || this.code === 21) {
      if (offset === 0) {
        this.advertised = byte
        await this.field('byte-count', byte, at)
      } else await this.fileByte(byte, at, response)
      return
    }
    if (this.code === 43) {
      await this.meiByte(byte, at, response, offset)
      return
    }
    const field = wordField(this.code, response, offset)
    if (field !== undefined || this.pending !== undefined) {
      await this.word(byte, at, field)
      return
    }
    const countAt = byteCountOffset(this.code, response)
    if (countAt !== undefined && offset === countAt) {
      this.advertised = byte
      await this.field('byte-count', byte, at)
      return
    }
    if (response && this.code === 7 && offset === 0) {
      await this.field('status', byte, at)
    } else if (response && this.code === 17 && offset === 1) {
      await this.field('server-id', byte, at)
    } else if (response && this.code === 17 && offset === 2) {
      await this.field('run-indicator', byte, at)
    } else if (response && this.code === 12 && offset >= 7) {
      await this.field('event', byte, at)
    } else if (
      (response && this.code === 24 && offset >= 4) ||
      (response && (this.code === 3 || this.code === 4 || this.code === 23) && offset >= 1) ||
      (!response && this.code === 16 && offset >= 5) ||
      (!response && this.code === 23 && offset >= 9)
    ) {
      await this.word(byte, at, undefined)
    } else if (
      (response && (this.code === 1 || this.code === 2) && offset >= 1) ||
      (!response && this.code === 15 && offset >= 5)
    ) {
      const first: u32 = response ? 1 : 5
      await this.out.emit(
        { kind: 'bits', frame: this.frame, offset: (offset - first) * 8, value: byte },
        at,
      )
    } else {
      await this.out.emit({ kind: 'data', frame: this.frame, offset, value: byte }, at)
    }
  }

  /** File records stream a seven-byte descriptor or one read subresponse at a time. */
  async fileByte(byte: u8, at: Provenance, response: bool): Promise<void> {
    if (this.remaining > 0) {
      await this.word(byte, at, undefined)
      this.remaining -= 1
      if (this.remaining === 0) this.subOffset = 0
      return
    }
    const offset = this.subOffset
    this.subOffset += 1
    if (this.code === 20 && response) {
      if (offset === 0) {
        await this.field('subresponse-length', byte, at)
        if (byte < 3 || (byte & 1) === 0) this.malformed = true
        this.remaining = byte > 0 ? byte - 1 : 0
      } else {
        await this.field('reference-type', byte, at)
        if (byte !== 6) this.malformed = true
      }
      // The reference byte precedes the register data, not part of remaining.
      if (offset === 0) this.remaining = 0
      else this.remaining = this.recordLength
      if (offset === 0) this.recordLength = byte > 0 ? byte - 1 : 0
      return
    }
    if (offset === 0) {
      await this.field('reference-type', byte, at)
      if (byte !== 6) this.malformed = true
    } else {
      let field: FieldName | undefined = undefined
      if (offset === 1) field = 'file-number'
      if (offset === 3) field = 'record-number'
      if (offset === 5) field = 'record-length'
      await this.word(byte, at, field)
      if (offset === 6) {
        this.subOffset = 0
        if (this.code === 21) this.remaining = this.recordLength * 2
      }
    }
  }

  /** MEI 14 headers and object lengths need only bounded scalar state. */
  async meiByte(byte: u8, at: Provenance, response: bool, offset: u32): Promise<void> {
    if (offset === 0) {
      this.mei = byte
      await this.field('mei-type', byte, at)
      if (byte !== 14) this.known = false
      return
    }
    if (this.mei !== 14) {
      if (this.frame !== undefined)
        await this.out.emit({ kind: 'data', frame: this.frame, offset, value: byte }, at)
      return
    }
    if (offset === 1) {
      await this.field('read-device-id', byte, at)
      if (byte < 1 || byte > 4) this.malformed = true
    } else if (!response && offset === 2) await this.field('object-id', byte, at)
    else if (response && offset === 2) {
      await this.field('conformity', byte, at)
      const level: u8 = byte & 0x7f
      if (level < 1 || level > 3) this.malformed = true
    } else if (response && offset === 3) {
      await this.field('more-follows', byte, at)
      if (byte !== 0 && byte !== 0xff) this.malformed = true
    } else if (response && offset === 4) await this.field('next-object-id', byte, at)
    else if (response && offset === 5) {
      this.objectCount = byte
      await this.field('object-count', byte, at)
    } else if (response && offset >= 6) {
      if (this.remaining > 0) {
        if (this.frame !== undefined)
          await this.out.emit({ kind: 'data', frame: this.frame, offset, value: byte }, at)
        this.remaining -= 1
      } else if (this.subOffset === 0) {
        this.objects += 1
        await this.field('object-id', byte, at)
        this.subOffset = 1
      } else {
        await this.field('object-length', byte, at)
        this.remaining = byte
        this.subOffset = 0
      }
    } else this.malformed = true
  }

  lengthOk(): bool {
    if (this.malformed || this.pending !== undefined) return false
    if (this.exception) return this.config.direction === 'response' && this.count === 1
    if (this.code === 43) {
      if (this.mei !== 14) return false
      return this.config.direction === 'response'
        ? this.count >= 6 &&
            this.remaining === 0 &&
            this.subOffset === 0 &&
            this.objects === this.objectCount
        : this.count === 3
    }
    if (this.code === 24)
      return this.config.direction === 'response'
        ? this.fifoCount <= 31 &&
            this.fifoBytes === 2 + this.fifoCount * 2 &&
            this.count === this.fifoBytes + 2
        : this.count === 2
    const response = this.config.direction === 'response'
    const countAt = byteCountOffset(this.code, response)
    if (countAt !== undefined) {
      const advertised = this.advertised
      if (advertised === undefined) return false
      if (this.count !== countAt + 1 + advertised) return false
      if (this.code === 20 || this.code === 21)
        return (
          advertised >= (response && this.code === 20 ? 4 : 7) &&
          this.remaining === 0 &&
          this.subOffset === 0
        )
      if (response && (this.code === 3 || this.code === 4 || this.code === 23))
        return advertised > 0 && (advertised & 1) === 0
      if (!response && (this.code === 15 || this.code === 16)) {
        const quantity = this.quantity
        if (quantity === undefined) return false
        if (this.code === 15)
          return quantity >= 1 && quantity <= 1968 && advertised === (quantity + 7) / 8
        return quantity >= 1 && quantity <= 123 && advertised === quantity * 2
      }
      if (!response && this.code === 23) {
        const readQuantity = this.readQuantity
        const writeQuantity = this.writeQuantity
        if (readQuantity === undefined) return false
        if (writeQuantity === undefined) return false
        return (
          readQuantity >= 1 &&
          readQuantity <= 125 &&
          writeQuantity >= 1 &&
          writeQuantity <= 121 &&
          advertised === writeQuantity * 2
        )
      }
      if (response && this.code === 12) return advertised >= 6 && advertised <= 70
      if (response && this.code === 17) return advertised >= 2
      return advertised > 0
    }
    if (
      this.code === 5 ||
      this.code === 6 ||
      this.code === 11 ||
      this.code === 15 ||
      this.code === 16
    )
      return this.count === (response || this.code === 5 || this.code === 6 ? 4 : 0)
    if (this.code === 22) return this.count === 6
    if (this.code === 7) return this.count === (response ? 1 : 0)
    if (this.code === 8) return this.count >= 4 && (this.count & 1) === 0
    if (!response && (this.code === 1 || this.code === 2 || this.code === 3 || this.code === 4)) {
      const max: u16 = this.code <= 2 ? 2000 : 125
      const quantity = this.quantity
      if (quantity === undefined) return false
      return this.count === 4 && quantity >= 1 && quantity <= max
    }
    if (!response && (this.code === 12 || this.code === 17)) return this.count === 0
    return !this.known
  }

  override async run(): Promise<void> {
    for (;;) {
      const received = await this.input.next()
      const event = received.event
      const at = received.at
      if (event.kind === 'begin') {
        this.clear()
        this.frame = event.frame
        this.code = event.function === undefined ? 0 : event.function & 0x7f
        this.exception = event.exception
        this.partial = event.partial
        this.known = event.function !== undefined && supported(this.code)
        this.malformed = false
        this.count = 0
        this.advertised = undefined
        this.quantity = undefined
        this.readQuantity = undefined
        this.writeQuantity = undefined
        this.registerIndex = 0
        this.crcOk = false
        this.subOffset = 0
        this.remaining = 0
        this.recordLength = 0
        this.fifoBytes = 0
        this.fifoCount = 0
        this.mei = 0
        this.objectCount = 0
        this.objects = 0
        await this.out.emit(
          {
            kind: 'begin',
            frame: event.frame,
            address: event.address,
            function: event.function,
            direction: this.config.direction,
            partial: event.partial,
          },
          at,
        )
      } else if (event.kind === 'data' && this.frame === event.frame) {
        if (event.offset !== this.count) this.malformed = true
        for (const byte of event.bytes) await this.byte(byte, at)
      } else if (event.kind === 'crc' && this.frame === event.frame) {
        this.crcOk = event.ok
      } else if (event.kind === 'end' && this.frame === event.frame) {
        const lengthOk = this.lengthOk()
        await this.out.emit(
          {
            kind: 'end',
            frame: event.frame,
            reason: event.reason,
            lengthOk,
            supported: this.known,
            valid:
              this.known && lengthOk && this.crcOk && !this.partial && event.reason === 'complete',
          },
          at,
        )
        this.clear()
      }
    }
  }
}

export const modbusPdu = defineStreamDecoder({
  name: 'modbus-pdu',
  category: 'device',
  config: { defaults: { direction: 'request' } },
  input: modbusEvents,
  output: modbusPduEvents,
  locales: {
    'en-US': {
      name: 'Modbus application protocol',
      summary: 'Function fields above RTU; configure request or response direction',
      fields: { direction: 'Direction' },
      values: { direction: { request: 'Client request', response: 'Server response' } },
      events: {
        begin: 'Begin',
        field: 'Function field',
        register: 'Register',
        bits: 'Coil / input bits',
        data: 'Data',
        end: 'End',
      },
      eventFields: {
        frame: 'Frame',
        address: 'Server address',
        function: 'Function code',
        direction: 'Direction',
        partial: 'Partial',
        field: 'Field',
        value: 'Value',
        index: 'Register index',
        offset: 'Offset',
        reason: 'Transport end',
        lengthOk: 'PDU length OK',
        supported: 'Function supported',
        valid: 'Validated PDU',
      },
    },
    'zh-CN': {
      name: 'Modbus 应用协议',
      summary: 'RTU 之上的功能字段；需指定请求或响应方向',
      fields: { direction: '方向' },
      values: { direction: { request: '客户端请求', response: '服务端响应' } },
      events: {
        begin: '开始',
        field: '功能字段',
        register: '寄存器',
        bits: '线圈 / 输入位',
        data: '数据',
        end: '结束',
      },
      eventFields: {
        frame: '帧',
        address: '服务端地址',
        function: '功能码',
        direction: '方向',
        partial: '不完整',
        field: '字段',
        value: '值',
        index: '寄存器索引',
        offset: '偏移',
        reason: '传输结束原因',
        lengthOk: 'PDU 长度正确',
        supported: '支持的功能',
        valid: 'PDU 已验证',
      },
    },
  },
  decoder: ModbusPdu,
})
