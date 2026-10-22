<!-- SPDX-License-Identifier: Apache-2.0 -->

# adf435x

The authoritative implementation is `dsl/decoders/adf435x.ts`. Its schema, configuration defaults and localized labels are generated from that source.

ADF4351 32-bit latch words, INT/FRAC/MOD/phase/prescaler, reference counter/doubler/divider, charge pump, RF divider/output controls.

The remaining scope is: Other ADF435x variants, computed frequency without reference/complete state, analog lock behavior and validation of reserved bits.

Reference: [ADF4351](https://www.analog.com/media/en/technical-documentation/data-sheets/adf4351.pdf).

Verification: Independent INT120/FRAC17 vector. Test execution is recorded in the [SPI coverage ledger](spi-porting-coverage.md); authored vectors are not a passing-test claim.

Events carry sample provenance. Gaps discard incomplete fields; complete previously emitted fields remain observations. Buffers are bounded and no whole transaction is retained. See the coverage ledger for input word widths, capture wiring and shared limitations.

The expanded ADF4351 profile emits every nonreserved field in registers R0–R5 (manufacturer figures 24–29), including phase adjust, noise/MUXOUT, lock detect, detector polarity, three-state/reset/double-buffer control, clock divider/fast-lock/resync, cycle-slip reduction, charge cancellation, antibacklash, auxiliary output and LD pin mode. Fields are observed latch requests, not evidence that a PLL locked or that double-buffered values were applied. Other ADF435x layouts are not inferred from the node family name.
