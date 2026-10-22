<!-- SPDX-License-Identifier: Apache-2.0 -->

# rfm12

The authoritative implementation is `dsl/decoders/rfm12.ts`. Its schema, configuration defaults and localized labels are generated from that source.

RFM12B named commands, power flags, frequency code, data-rate calculation, RX bandwidth/LNA/RSSI, TX deviation/power, FIFO bytes and returned status bits.

The remaining scope is: Other RFM12 variants, analog calibration and over-air framing. Nominal carrier frequency requires an observed band selection.

Reference: [HopeRF RFM12B](https://www.hoperf.com/uploads/RFM12BDatasheet_1695367948.pdf).

Verification: All seventeen command classes, returned status and state-dependent frequency/wake vectors pass with generated/interpreted event and provenance comparisons. Test checkpoints are recorded in the [SPI coverage ledger](spi-porting-coverage.md).

Events carry sample provenance. Gaps discard incomplete fields; complete previously emitted fields remain observations. Buffers are bounded and no whole transaction is retained. See the coverage ledger for input word widths, capture wiring and shared limitations.

All seventeen documented command classes now expose parameters, including crystal loading and band selection; filter/clock recovery; FIFO/reset/sync; AFC mode/range/strobe/fine/enable; PLL control; wake mantissa/exponent; low-duty-cycle and low-battery/clock codes. Observed band selection permits nominal carrier frequency; observed wake mantissa permits duty-cycle basis points. A gap forgets those dependencies. Wake periods use a u64 microsecond event to avoid overflow. Analog calibration and over-air framing remain outside the profile.
