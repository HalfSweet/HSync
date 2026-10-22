// SPDX-License-Identifier: Apache-2.0
/** Exact native sensor contract. Five bytes are required, never a bounded vector. */
import type { bool, u8, u16 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type ErrorReason = 'start' | 'response' | 'bit' | 'count'
export type Am230xEvent =
  | {
      kind: 'reading'
      bytes: [u8, u8, u8, u8, u8]
      checksumOk: bool
      humidityRaw: u16
      temperatureRaw: u16
    }
  | { kind: 'error'; reason: ErrorReason }
export const am230xEvents = eventSchema<Am230xEvent>('am230x.events', 1).roles({
  reading: { role: 'data', errorWhen: { checksumOk: false } },
  error: 'framing',
})
