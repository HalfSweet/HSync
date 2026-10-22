// SPDX-License-Identifier: Apache-2.0
/* eslint-disable @typescript-eslint/no-unnecessary-type-assertion -- These casts exercise DSL integer-width semantics that differ from TypeScript number semantics. */

/** Backend acceptance source; kept outside the production decoder/catalog roots. */
import type { Channel, bool, u8, u16, u64 } from 'uiharu'
import { LogicDecoder, atLeast, defineLogicDecoder, mask, one } from 'uiharu'
import {
  probeEvents,
  type Ack,
  type Clause,
  type Five,
  type Four,
  type ProbeEvent,
  type Version,
} from './backend-contracts'

const FIXED_UNIT: Ack = 'wait'
const FIXED_CLAUSE: Clause = 45

export interface ProbeConfig {
  channel: Channel
  four: Four
  five: Five
  enabled: bool
  clause: Clause
  ack: Ack
  version: Version
}

function validate(config: ProbeConfig): string | undefined {
  if (config.channel > 0) return `clause ${config.clause}; version ${config.version}`
  return undefined
}

function switches(): u64 {
  let total: u64 = 0
  outer: for (let i: u64 = 0; i < 4; i++) {
    switch (i) {
      case 0:
        if (i === 0) break
        total += 100
        break
      case 1:
        if (i === 1) continue outer
        total += 100
        break
      case 3:
        if (i === 3) continue
        total += 100
        break
      default:
        for (let j: u64 = 0; j < 2; j++) {
          if (j === 0) break
          total += 100
        }
        switch (i) {
          default:
            total += 1000
            break
          case 2:
            if (i === 2) break
            total += 100
            break
        }
        total += 10
        break
    }
    total += 1
  }
  return total
}

function rangeCast(): u64 {
  let total: u64 = 0
  for (let bit: u8 = 0; bit < 3; bit++) {
    total += (1 << (bit as u64)) as u8
  }
  for (let shift = 0; shift < 3; shift++) {
    total += shift as u8
  }
  return total
}

function optionalLiteralWidth(): u64 {
  let total: u64 = 0
  for (const value of [300, undefined]) {
    if (value !== undefined) total += (value / 4) as u8
  }
  return total
}

function optionalConditionalWidth(flag: bool): u64 {
  const value = flag ? 300 : undefined
  if (value !== undefined) return (value / 4) as u8
  return 0
}

function ackCode(ack: Ack): u16 {
  if (typeof ack === 'string') {
    switch (ack) {
      case 'ok':
        return 1
      case 'wait':
        return 2
      case 'fault':
        return 3
    }
  } else if ('other' in ack) {
    return 100 + ack.other
  }
  return 0
}

function isOk(ack: Ack): bool {
  return ack === 'ok'
}

function clauseCode(clause: Clause): u16 {
  switch (clause) {
    case 22:
      return 220
    case 45:
      return 450
  }
}

class Probe extends LogicDecoder<ProbeConfig, ProbeEvent> {
  readonly watch = mask(this.config.channel)
  override async run(): Promise<void> {
    for (;;) {
      const edge = await this.signal.nextChange(this.watch)
      const small: u8 = 15
      const wide: u16 = 300
      let fixedSum: u16 = 0
      for (const value of this.config.four) fixedSum += value
      const optional: Four | undefined = this.config.enabled ? [9, 8, 7, 6] : undefined
      const address: u64 = ((0xfedcba98 as u64) << 32) | 0x76543210
      await this.out.emit(
        {
          kind: 'probe',
          four: [1, 2, 3, 4],
          five: [5, 6, 7, 8, 9],
          optional,
          address: address + 1,
          clause: this.config.clause,
          constantClause: FIXED_CLAUSE,
          ack: this.config.ack,
          constructed: { other: 7 },
          unit: FIXED_UNIT,
          version: this.config.version,
          fixedSum,
          last: this.config.five[4],
          switchCount: switches(),
          rangeCast: rangeCast(),
          optionalWidth: optionalLiteralWidth(),
          conditionalWidth: optionalConditionalWidth(this.config.enabled),
          nestedWide: small * 100 + wide,
          castWide: (1 << 40) as u64,
          castBarrier: (small * 100) as u8 as u64,
          ternaryCast: (this.config.enabled ? wide : wide + 1) as u8,
          countWidth: 1 << (8 as u8),
          contextualMask: wide & ((1 << small) - 1),
          compareWide: small * 100 < wide,
          literalTree: (2 * 3 + small) as u8,
          ackCode: ackCode(this.config.ack),
          isOk: isOk(this.config.ack),
          clauseCode: clauseCode(this.config.clause),
          isC22: this.config.clause === 22,
        },
        one(edge.tick),
      )
    }
  }
}

export const probe = defineLogicDecoder({
  name: 'backend-probe',
  category: 'line',
  config: {
    defaults: {
      channel: 0,
      four: [1, 2, 3, 4],
      five: [5, 6, 7, 8, 9],
      enabled: true,
      clause: 22,
      ack: 'ok',
      version: '1.2',
    },
    validate,
  },
  props: (config) => ({ clause: config.clause }),
  requires: (config) => ({ channels: atLeast(config.channel + 1) }),
  output: probeEvents,
  decoder: Probe,
})
