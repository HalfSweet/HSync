// SPDX-License-Identifier: Apache-2.0
/** Exact network wire contract, including hexadecimal ROM IDs. */
import type { HexU64, bool, u64, u8 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type TxId = u64
export type EndReason = 'reset' | 'bad-slot' | 'aborted'
export type OneWireEvent =
  | { kind: 'begin'; tx: TxId; presence: bool }
  | { kind: 'rom-command'; tx: TxId; code: u8 }
  | { kind: 'rom'; tx: TxId; id: HexU64; family: u8; crcOk: bool }
  | { kind: 'search'; tx: TxId; id: HexU64; crcOk: bool; conflicts: u8; unanswered: u8 }
  | { kind: 'byte'; tx: TxId; value: u8 }
  | { kind: 'bits'; tx: TxId; value: u8; count: u8 }
  | { kind: 'end'; tx: TxId; reason: EndReason }
export const onewireEvents = eventSchema<OneWireEvent>('onewire.events', 1).roles({
  begin: 'framing',
  'rom-command': 'command',
  rom: { role: 'address', errorWhen: { crcOk: false } },
  search: { role: 'address', errorWhen: { crcOk: false } },
  byte: 'data',
  bits: 'data',
  end: 'framing',
})
