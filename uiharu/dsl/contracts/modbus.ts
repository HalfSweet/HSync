// SPDX-License-Identifier: Apache-2.0

/** The existing RTU transport JSON contract; declared here for application-layer consumers. */
import type { Bounded, bool, u16, u32, u64, u8 } from 'uiharu'
import { eventSchema } from 'uiharu'

export type FrameId = u64
export type EndReason = 'complete' | 'bad-crc' | 'short' | 'broken' | 'aborted'
export type ModbusEvent =
  | {
      kind: 'begin'
      frame: FrameId
      address: u8
      function: u8 | undefined
      exception: bool
      partial: bool
    }
  | { kind: 'data'; frame: FrameId; offset: u32; bytes: Bounded<u8, 32> }
  | { kind: 'crc'; frame: FrameId; value: u16; ok: bool }
  | { kind: 'end'; frame: FrameId; reason: EndReason }
export const modbusEvents = eventSchema<ModbusEvent>('modbus.events', 1).roles({
  begin: 'address',
  data: 'data',
  crc: { role: 'check', errorWhen: { ok: false } },
  end: 'framing',
})
