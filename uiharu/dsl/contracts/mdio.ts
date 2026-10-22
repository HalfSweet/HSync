// SPDX-License-Identifier: Apache-2.0
/** Exact IEEE 802.3 native frame contract; clause remains a validated numeric enum. */
import type { NumericEnum, u8, u16 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type Clause = NumericEnum<{ c22: 22; c45: 45 }>
export type Op = 'read' | 'write' | 'address' | 'post-read-increment'
export type ErrorReason = 'start' | 'turnaround' | 'aborted'
export type MdioEvent =
  | {
      kind: 'frame'
      clause: Clause
      op: Op
      phy: u8
      reg: u8
      turnaround: u8
      data: u16
      preambleBits: u8
    }
  | { kind: 'error'; reason: ErrorReason }
export const mdioEvents = eventSchema<MdioEvent>('mdio.events', 1).roles({
  frame: 'data',
  error: 'framing',
})
