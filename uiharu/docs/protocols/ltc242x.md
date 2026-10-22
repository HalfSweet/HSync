<!-- SPDX-License-Identifier: Apache-2.0 -->

# ltc242x

The authoritative implementation is `dsl/decoders/ltc242x.ts`. Its schema, configuration defaults and localized labels are generated from that source.

LTC2420/21/22 24-bit EOC/sign/extended-range/channel, signed µV magnitude and explicit reference.

The remaining scope is: Other LTC24xx frame layouts and physical conversion timing.

Reference: [LTC2420](https://www.analog.com/media/en/technical-documentation/data-sheets/2420f.pdf), [LTC2421/22](https://www.analog.com/media/en/technical-documentation/data-sheets/24212f.pdf).

Verification: Independent positive/negative microvolt and channel vectors pass with generated/interpreted event and provenance comparisons. Test checkpoints are recorded in the [SPI coverage ledger](spi-porting-coverage.md).

Events carry sample provenance. Gaps discard incomplete fields; complete previously emitted fields remain observations. Buffers are bounded and no whole transaction is retained. See the coverage ledger for input word widths, capture wiring and shared limitations.
