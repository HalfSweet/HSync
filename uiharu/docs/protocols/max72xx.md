<!-- SPDX-License-Identifier: Apache-2.0 -->

# max72xx

The authoritative implementation is `dsl/decoders/max72xx.ts`. Its schema, configuration defaults and localized labels are generated from that source.

Same explicitly scoped MAX7219/MAX7221 profile.

The remaining scope is: Other MAX72xx devices.

Reference: Same specification.

Verification: Separate-node shutdown and display-test vectors pass with generated/interpreted event and provenance comparisons. Test checkpoints are recorded in the [SPI coverage ledger](spi-porting-coverage.md).

Events carry sample provenance. Gaps discard incomplete fields; complete previously emitted fields remain observations. Buffers are bounded and no whole transaction is retained. See the coverage ledger for input word widths, capture wiring and shared limitations.
