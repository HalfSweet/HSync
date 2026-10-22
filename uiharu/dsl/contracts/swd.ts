// SPDX-License-Identifier: Apache-2.0
/** Canonical SWD transport DTO, preserving swd.events@1 wire JSON. */
import type { bool, u8, u32, u64 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type Ack = 'ok' | 'wait' | 'fault' | { other: u8 }
export type EndReason = 'complete' | 'wait' | 'fault' | 'protocol-error' | 'aborted'
export type Interface = 'swd' | 'jtag' | 'dormant'
export type SwdEvent =
  | { kind: 'begin'; tx: u64 }
  | { kind: 'request'; tx: u64; apndp: bool; rnw: bool; addr: u8; parityOk: bool }
  | { kind: 'ack'; tx: u64; value: Ack }
  | { kind: 'data'; tx: u64; value: u32; parityOk: bool; fromTarget: bool }
  | { kind: 'end'; tx: u64; reason: EndReason }
  | { kind: 'line-reset' }
  | { kind: 'switch'; to: Interface }
export const swdEvents = eventSchema<SwdEvent>('swd.events', 1).roles({
  begin: 'framing',
  request: 'command',
  ack: 'handshake',
  data: 'data',
  end: 'framing',
  'line-reset': 'line',
  switch: 'control',
})
