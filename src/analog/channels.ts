/**
 * Analog channels share the index space of logic channels from `hsync_core::ANALOG_BASE` up, so
 * that one index names any channel: its colour, height and name are kept like a logic
 * channel's.
 */

/** Index of analog channel A0; A`n` is `ANALOG_BASE + n`. */
export const ANALOG_BASE = 0x8000

/** Whether `index` names an analog channel. */
export function isAnalog(index: number): boolean {
  return index >= ANALOG_BASE
}

/** How an analog channel is numbered on the timeline, e.g. `A0`. */
export function analogNumber(index: number): string {
  return `A${index - ANALOG_BASE}`
}

/** Bits a sample of an analog channel takes in a capture: its code, 16 bits wide. */
export const ANALOG_SAMPLE_BITS = 16
