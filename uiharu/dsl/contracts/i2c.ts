// SPDX-License-Identifier: Apache-2.0

/**
 * The `i2c.events` contract: what the I²C decoder emits and what decoders built on I²C read.
 * The decoder itself is written by hand (`crates/uiharu-protocols/src/i2c/`); this module
 * declares the contract, so that its Rust types are generated and a decoder written in the DSL
 * can stack on the bus.
 *
 * A transaction runs from a START to a STOP and is numbered from 1 by the decoder; a repeated
 * START stays in it, as a `restart` event, and the address after it is another `address`
 * event. A transaction found under way at the first sample or after a gap begins `partial`:
 * its bytes cannot be aligned, so it carries none, and it ends at the STOP or where the next
 * START begins the next transaction (reason `restart`). A gap or the end of the samples ends
 * the open transaction `aborted`. The rules of the decoder are in `docs/protocols/i2c.md`.
 */

import type { bool, u16, u64, u8 } from 'uiharu'
import { eventSchema } from 'uiharu'

/** A transaction: from a START to a STOP. */
export type TxId = u64

/** Why a transaction ended. */
export type EndReason =
  /** A STOP condition: the transaction is whole. */
  | 'stop'
  /** A START condition ended a partial transaction; the transaction it begins follows at the
   * same tick. Only partial transactions end this way: in any other, a repeated START is a
   * `restart` event and the transaction goes on. */
  | 'restart'
  /** The samples ended or were lost while the transaction was under way. */
  | 'aborted'

/** What travels on an I²C event stream. */
export type I2cEvent =
  /** A START condition, or a transaction found under way (`partial`: it was already busy when
   * the samples began, or was clocked with no START seen; it carries no bytes). */
  | { kind: 'begin'; tx: TxId; partial: bool }
  /** The address after a START or repeated START: 7-bit or 10-bit (`tenBit`), the R/W̄ bit
   * (`read`: the controller reads from the target), and whether it was acknowledged — for a
   * 10-bit write address, both its bytes. */
  | { kind: 'address'; tx: TxId; address: u16; tenBit: bool; read: bool; ack: bool }
  /** A data byte, most significant bit first on the bus, and whether the receiver acknowledged
   * it: SDA low on the ninth clock. */
  | { kind: 'data'; tx: TxId; byte: u8; ack: bool }
  /** A repeated START: the transaction goes on with another address. `droppedBits` are the bits
   * of an unfinished byte it cut short; they make no byte. */
  | { kind: 'restart'; tx: TxId; droppedBits: u8 }
  /** The transaction ended, `droppedBits` of an unfinished byte cut short. */
  | { kind: 'end'; tx: TxId; reason: EndReason; droppedBits: u8 }

/** The schema of I²C event streams: a transaction's items are its addresses, data bytes and
 * repeated STARTs; a STOP ends it normally, as does the START of the next transaction when the
 * transaction was partial. */
export const i2cEvents = eventSchema<I2cEvent>('i2c.events', 1).framed({
  begin: 'begin',
  items: ['address', 'data', 'restart'],
  end: 'end',
  id: 'tx',
  partial: 'partial',
  reason: 'reason',
  normal: ['stop', 'restart'],
})
