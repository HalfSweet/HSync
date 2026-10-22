// SPDX-License-Identifier: Apache-2.0

/** Deliberately difficult wire types shared by two independent test producers. */
import type { HexU64, NumericEnum, bool, u8, u16, u64 } from 'uiharu'
import { eventSchema } from 'uiharu'

export type Four = [u8, u8, u8, u8]
export type Five = [u8, u8, u8, u8, u8]
export type Clause = NumericEnum<{ c22: 22; c45: 45 }>
export type Ack = 'ok' | 'wait' | 'fault' | { other: u8 }
export type Version = '1.2' | '2.0' | 'foo-bar' | 'foo_bar' | 'FooBar' | 'Self'
export type ProbeEvent = {
  kind: 'probe'
  four: Four
  five: Five
  optional: Four | undefined
  address: HexU64
  clause: Clause
  constantClause: Clause
  ack: Ack
  constructed: Ack
  unit: Ack
  version: Version
  fixedSum: u16
  last: u8
  switchCount: u64
  rangeCast: u64
  optionalWidth: u64
  conditionalWidth: u64
  nestedWide: u16
  castWide: u64
  castBarrier: u64
  ternaryCast: u8
  countWidth: u16
  contextualMask: u16
  compareWide: bool
  literalTree: u8
  ackCode: u16
  isOk: bool
  clauseCode: u16
  isC22: bool
}
export const probeEvents = eventSchema<ProbeEvent>('backend-probe.events', 1).roles({
  probe: 'data',
})
