<!-- SPDX-License-Identifier: Apache-2.0 -->

# adxl345-spi

The authoritative implementation is `dsl/decoders/adxl345-spi.ts`. Its schema, configuration defaults and localized labels are generated from that source.

Multibyte register reads/writes, signed little-endian XYZ raw samples and named rate/power/range/interrupt/FIFO controls.

The remaining scope is: calibrated physical acceleration, orientation and interrupt timing.

Reference: [Analog Devices ADXL345 revG](https://www.analog.com/media/en/technical-documentation/data-sheets/ADXL345.pdf).

Verification: Independent -1/+256/-32768 axes and queue/block/budget invariance. Test execution is recorded in the [SPI coverage ledger](spi-porting-coverage.md); authored vectors are not a passing-test claim.

Events carry sample provenance. Gaps discard incomplete fields; complete previously emitted fields remain observations. Buffers are bounded and no whole transaction is retained. See the coverage ledger for input word widths, capture wiring and shared limitations.

Registers 1Dh–39h include tap/activity/free-fall thresholds, signed offsets, tap duration/latency/window, inactivity/free-fall timing, axis control/status, rate, power, data format and FIFO fields. Engineering units use integer micro-g, microseconds, milliseconds, seconds or milli-Hz as named by the field. Thresholds use 62.5 mg/LSB; offsets use 15.6 mg/LSB.

Axis micro-g values require an observed DATA_FORMAT read/write. They account for full/fixed resolution, selected range and left/right justification, using the nominal 256 LSB/g full-resolution sensitivity (scaled for fixed resolution). The scalar magnitude and negative flag preserve signed values; integer conversion truncates fractional micro-g. Unknown initial format and gaps never invent a scale. These are nominal data-sheet conversions, not calibration measurements.
