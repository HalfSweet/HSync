// SPDX-License-Identifier: Apache-2.0

/** Exact native ps2.events@1 wire contract. The physical producer retains its
 * signal-level implementation; these generated types allow a DSL consumer. */
import type { bool, u8 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type Direction = 'device-to-host' | 'host-to-device'
export type ErrorReason = 'start' | 'aborted'
export type Ps2Event =
  | {
      kind: 'frame'
      direction: Direction
      value: u8
      parityOk: bool
      stopOk: bool
      ack: bool | undefined
    }
  | { kind: 'error'; reason: ErrorReason }
export const ps2Events = eventSchema<Ps2Event>('ps2.events', 1).roles({
  frame: { role: 'data', errorWhen: [{ parityOk: false }, { stopOk: false }, { ack: false }] },
  error: 'framing',
})
