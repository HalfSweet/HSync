// SPDX-License-Identifier: Apache-2.0
/** Exact native DMX transport contract, without interpreting alternate start codes. */
import type { Bounded, u8, u32, u64 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type EndReason = 'break' | 'aborted'
export type Dmx512Event =
  | { kind: 'packet-begin'; packet: u64; breakTicks: u64 }
  | { kind: 'start-code'; packet: u64; code: u8 }
  | { kind: 'slots'; packet: u64; offset: u32; values: Bounded<u8, 32> }
  | { kind: 'packet-end'; packet: u64; slots: u32; reason: EndReason }
  | { kind: 'error'; packet: u64; slot: u32 }
export const dmx512Events = eventSchema<Dmx512Event>('dmx512.events', 1).roles({
  'packet-begin': 'framing',
  'start-code': 'command',
  slots: 'data',
  'packet-end': 'framing',
  error: 'error',
})
