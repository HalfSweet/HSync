/** Bus speeds of I²C: standard, fast, fast-mode plus, high-speed and ultra-fast mode. */
export const I2C_RATES = [100e3, 400e3, 1e6, 3.4e6, 5e6]

/** The nominal rate `hz` is within `tolerance` of, if any. */
export function nominalRate(hz: number, rates: readonly number[], tolerance = 0.03): number | null {
  return rates.find((rate) => Math.abs(hz - rate) / rate <= tolerance) ?? null
}
