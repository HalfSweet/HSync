// SPDX-License-Identifier: Apache-2.0

/** An independent output owner using the same imported schema and native event type. */
import type { bool } from 'uiharu'
import { StreamDecoder, defineStreamDecoder } from 'uiharu'
import { probeEvents, type ProbeEvent } from './backend-contracts'

export interface ForwardConfig {
  enabled: bool
}

class Forward extends StreamDecoder<ForwardConfig, ProbeEvent, ProbeEvent> {
  override async run(): Promise<void> {
    for (;;) {
      const received = await this.input.next()
      const event = received.event
      if (event.kind === 'probe' && this.config.enabled) {
        await this.out.emit(
          {
            kind: 'probe',
            four: event.four,
            five: event.five,
            optional: event.optional,
            address: event.address + 1,
            clause: event.clause,
            constantClause: event.constantClause,
            ack: event.ack,
            constructed: event.constructed,
            unit: event.unit,
            version: event.version,
            fixedSum: event.fixedSum,
            last: event.last,
            switchCount: event.switchCount,
            rangeCast: event.rangeCast,
            optionalWidth: event.optionalWidth,
            conditionalWidth: event.conditionalWidth,
            nestedWide: event.nestedWide,
            castWide: event.castWide,
            castBarrier: event.castBarrier,
            ternaryCast: event.ternaryCast,
            countWidth: event.countWidth,
            contextualMask: event.contextualMask,
            compareWide: event.compareWide,
            literalTree: event.literalTree,
            ackCode: event.ackCode,
            isOk: event.isOk,
            clauseCode: event.clauseCode,
            isC22: event.isC22,
          },
          received.at,
        )
      }
    }
  }
}

export const forward = defineStreamDecoder({
  name: 'backend-forward',
  category: 'line',
  config: { defaults: { enabled: true } },
  input: probeEvents,
  output: probeEvents,
  decoder: Forward,
})
