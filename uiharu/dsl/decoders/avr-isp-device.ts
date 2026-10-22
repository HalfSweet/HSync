// SPDX-License-Identifier: Apache-2.0
/** AVR signature observations from Microchip AT15007 §3.6. No GPL tables or
 * implementation copied. Additional part signatures remain numeric observations. */
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- Integer assertions select IR widths and truncation. */
import type { u8, u16, u32, u64, Range, Tick, Provenance } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema, join } from 'uiharu'
import { ispEvents, type IspEvent } from '../contracts/avr-isp'
// eslint-disable-next-line @typescript-eslint/no-empty-object-type -- Exact empty DSL configuration.
export interface AvrIspDeviceConfig {}
export type Vendor = 'atmel-microchip' | 'unknown'
export type Device = 'atmega328' | 'atmega328p' | 'atmega328pb' | 'unknown'
export type Event =
  | { kind: 'signature-byte'; op: u64; tx: u64; index: u16; value: u8 }
  | {
      kind: 'signature'
      tx: u64
      value: u32
      manufacturer: u8
      family: u8
      part: u8
      vendor: Vendor
      device: Device
    }
  | { kind: 'sync-warning'; op: u64; tx: u64 }
export const events = eventSchema<Event>('avr-isp-device.events', 1).roles({
  'signature-byte': 'data',
  signature: 'address',
  'sync-warning': 'error',
})
class AvrIspDevice extends StreamDecoder<AvrIspDeviceConfig, IspEvent, Event> {
  heldTx: u64 | undefined = undefined
  index: u8 = 0
  value: u32 = 0
  first: Provenance | undefined = undefined
  clear(): void {
    this.heldTx = undefined
    this.index = 0
    this.first = undefined
    this.value = 0
  }
  override onGap(_range: Range): void {
    this.clear()
  }
  override onEnd(_tick: Tick): void {
    this.clear()
  }
  override hold(): Tick | undefined {
    const first = this.first
    return first?.span.start
  }
  override async run(): Promise<void> {
    for (;;) {
      const item = await this.input.next()
      const event = item.event
      if (event.kind === 'instruction') {
        if (event.command === 'programming-enable' && event.inSync === false)
          await this.out.emit({ kind: 'sync-warning', op: event.op, tx: event.tx }, item.at)
        if (event.command !== 'read-signature-byte') {
          this.clear()
          continue
        }
        const signatureIndex: u16 = event.address ?? 0xffff
        const signatureByte: u8 = event.dataOut ?? 0
        if (event.address === undefined || event.dataOut === undefined) {
          this.clear()
          continue
        }
        await this.out.emit(
          {
            kind: 'signature-byte',
            op: event.op,
            tx: event.tx,
            index: signatureIndex,
            value: signatureByte,
          },
          item.at,
        )
        if (signatureIndex === 0) {
          this.heldTx = event.tx
          this.index = 1
          this.value = signatureByte
          this.first = item.at
          continue
        }
        if (event.tx !== this.heldTx || signatureIndex !== this.index) {
          this.clear()
          continue
        }
        this.value = (this.value << 8) | signatureByte
        this.index += 1
        if (this.index !== 3) continue
        const signature = this.value
        const manufacturer: u8 = (signature >> 16) as u8
        const family: u8 = (signature >> 8) as u8
        const part: u8 = signature as u8
        const vendor: Vendor = manufacturer === 0x1e ? 'atmel-microchip' : 'unknown'
        const device: Device =
          signature === 0x1e9514
            ? 'atmega328'
            : signature === 0x1e950f
              ? 'atmega328p'
              : signature === 0x1e9516
                ? 'atmega328pb'
                : 'unknown'
        const first = this.first
        // Single-byte observations are emitted immediately; the aggregate settles at byte2.
        await this.out.emit(
          {
            kind: 'signature',
            tx: event.tx,
            value: signature,
            manufacturer,
            family,
            part,
            vendor,
            device,
          },
          first === undefined ? item.at : join(first, item.at),
        )
        this.clear()
      } else this.clear()
    }
  }
}
export const avrIspDevice = defineStreamDecoder({
  name: 'avr-isp-device',
  category: 'device',
  config: { defaults: {} },
  input: ispEvents,
  output: events,
  props: () => ({}),
  locales: {
    'en-US': {
      name: 'AVR ISP signatures',
      summary: 'Observed manufacturer and device signature with programming-enable echo warning',
      events: {
        'signature-byte': 'Signature byte',
        signature: 'Device signature',
        'sync-warning': 'Programming-enable echo mismatch',
      },
      eventFields: {
        op: 'Instruction',
        tx: 'Session',
        index: 'Signature address',
        value: 'Value',
        manufacturer: 'Manufacturer byte',
        family: 'Family byte',
        part: 'Part byte',
        vendor: 'Vendor',
        device: 'Device',
      },
    },
    'zh-CN': {
      name: 'AVR ISP签名',
      summary: '观测厂商和器件签名，并检查编程使能回显',
      events: {
        'signature-byte': '签名字节',
        signature: '器件签名',
        'sync-warning': '编程使能回显不符',
      },
      eventFields: {
        op: '指令',
        tx: '会话',
        index: '签名地址',
        value: '数值',
        manufacturer: '厂商字节',
        family: '系列字节',
        part: '型号字节',
        vendor: '厂商',
        device: '器件',
      },
    },
  },
  decoder: AvrIspDevice,
})
