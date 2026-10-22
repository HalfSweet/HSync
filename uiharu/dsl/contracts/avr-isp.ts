// SPDX-License-Identifier: Apache-2.0
/** Exact native AVR ISP contract: complete instructions carry four MOSI bytes. */
import type { bool, u8, u16, u64, Bounded } from 'uiharu'
import { eventSchema } from 'uiharu'
export type InstructionBytes = readonly [u8, u8, u8, u8]
export type Command =
  | 'programming-enable'
  | 'chip-erase'
  | 'poll-rdy-bsy'
  | 'load-extended-address'
  | 'load-program-memory-page-high'
  | 'load-program-memory-page-low'
  | 'load-eeprom-memory-page'
  | 'read-program-memory-high'
  | 'read-program-memory-low'
  | 'read-eeprom-memory'
  | 'read-lock-bits'
  | 'read-signature-byte'
  | 'read-fuse-bits'
  | 'read-fuse-high-bits'
  | 'read-extended-fuse-bits'
  | 'read-calibration-byte'
  | 'write-program-memory-page'
  | 'write-eeprom-memory'
  | 'write-eeprom-memory-page'
  | 'write-lock-bits'
  | 'write-fuse-bits'
  | 'write-fuse-high-bits'
  | 'write-extended-fuse-bits'
export type ErrorReason = 'incomplete' | 'aborted'
export type IspEvent =
  | {
      kind: 'instruction'
      op: u64
      tx: u64
      command: Command
      address: u16 | undefined
      dataIn: u8 | undefined
      dataOut: u8 | undefined
      inSync: bool | undefined
      bytes: InstructionBytes
    }
  | { kind: 'unknown'; tx: u64; bytes: InstructionBytes }
  | { kind: 'error'; tx: u64; reason: ErrorReason; bytes: Bounded<u8, 3> }
export const ispEvents = eventSchema<IspEvent>('avrisp.events', 1).roles({
  instruction: 'command',
  unknown: 'data',
  error: 'error',
})
