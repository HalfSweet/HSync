// SPDX-License-Identifier: Apache-2.0

/** Existing single-TAP ADIv5 transport contract, preserving its JSON schema exactly. */
import type { bool, u32, u64, u8 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type Ack = 'wait' | 'ok-fault' | 'invalid'
export type ScanStatus =
  'wait' | 'aborted' | 'invalid-length' | 'unknown-instruction' | 'missing-line'
export type JtagDpEvent =
  | { kind: 'reset' }
  | { kind: 'instruction'; scan: u64; value: u8 }
  | { kind: 'idcode'; scan: u64; value: u32 }
  | { kind: 'bypass'; scan: u64; tdi: bool; tdo: bool }
  | { kind: 'abort'; scan: u64; value: u32; lowBits: u8 }
  | {
      kind: 'access'
      scan: u64
      instruction: u8
      read: bool
      addr: u8
      dataIn: u32
      ackBits: u8
      ack: Ack
      responseData: u32
      previousScan: u64 | undefined
      accepted: bool | undefined
    }
  | { kind: 'scan'; scan: u64; ir: bool; bits: u32; status: ScanStatus }
export const jtagDpEvents = eventSchema<JtagDpEvent>('jtag-dp.events', 1).roles({
  reset: 'framing',
  instruction: 'command',
  idcode: 'data',
  bypass: 'data',
  abort: 'command',
  access: 'data',
  scan: 'framing',
})
