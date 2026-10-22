<!-- SPDX-License-Identifier: Apache-2.0 -->

# sd-native

The authoritative implementation is `dsl/decoders/sd-native.ts`. Its schema, configuration defaults and localized labels are generated from that source.

Existing native command transport; new SDR CMD, R2 CID/CSD v1/v2, one/four-bit DAT, read/write payload, per-lane CRC16, write response and DAT0 busy; explicit width/block bytes, observed CMD16.

The remaining scope is: DDR/UHS-II, tuning, MMC/eMMC, SDIO data, CSD v3; bus-width changes require observed accepted commands.

Reference: [SD Physical Layer Simplified v2.00](https://chlazza.nfshost.com/docs/Simplified_Physical_Layer_Spec.pdf) §§4.3–4.9,5.

Verification: CMD17/24 data, CRC fault, response/busy, invalid configs, gap/invariance. Test execution is recorded in the [SPI coverage ledger](spi-porting-coverage.md); authored vectors are not a passing-test claim.

Events carry sample provenance. Gaps discard incomplete fields; complete previously emitted fields remain observations. Buffers are bounded and no whole transaction is retained. See the coverage ledger for input word widths, capture wiring and shared limitations.

With `width: auto`, the capture starts in one-bit mode and changes only after a CRC-valid CMD55 response with APP_CMD set followed by a successful ACMD6 response. A plain CMD6, illegal-command response or gap cannot silently enable four-bit decoding. Auto mode requires all four DAT channels in the configuration. Explicit one/four settings preserve the selected width. Per-lane CRC16 is reported independently; card acceptance and DAT0 busy are separate observations.

ACMD51 is recognized only after a CRC-valid CMD55 response with APP_CMD set and a matching accepted ACMD51 response. Its data block is always eight bytes, independent of CMD16/configured block size, and follows the active one/four-lane transport. SCR events include the full block span, all-lane CRC validity, structure/specification/security/bus-width codes, erased-data state, extended flags and manufacturer bits from [SD v5.10 table5-17](https://www.sdcard.org/cms/wp-content/themes/sdcard-org/dl.php?f=Part1_Physical_Layer_Simplified_Specification_Ver5.10.pdf). These capability flags do not change the active width.

R1 state changes require the response command index to match the pending command. Wrong-index responses cannot activate a data block, authorize an application command or apply ACMD6. CMD0 restores one-bit width only in auto mode; explicit one/four settings remain fixed. Ten focused compiled/interpreted tests pass, including accepted/rejected SCR in one/four lanes, a CRC fault, wrong-index R1 and fixed-four-bit decoding after CMD0.

The P1 field audit closes the pinned native-SD reference's actual response/register
inventory. Matched R1/R1b responses now expose named card-status fields (including
state, readiness, APP_CMD and error flags). R6 separates the published RCA and
maps its abbreviated status bits back to their card-status meanings. R7 reports
accepted voltage and check pattern, with explicit comparisons against the last
CRC-valid request. Mismatched response indices remain raw command observations.
Native R3 exposes OCR readiness, capacity and voltage-window bits.

Both native R2 and SPI register consumers expose the exact five-byte CID product
name and all nonreserved CSDv1 fields named by the reference: timing/rate codes,
command classes, read/write alignment and lengths, current codes, capacity
multiplier, erase geometry, write-protect geometry, write-speed factor, format,
copy and protection flags, and register CRC. CSDv2 omits the obsolete v1 current,
multiplier and write-protect-group interpretations while retaining its capacity
and common fields. Raw timing/current codes are observations, not analog
measurements. Reserved values are not assigned invented meanings. These facts
follow [SD Physical Layer Simplified v2.00](https://chlazza.nfshost.com/docs/Simplified_Physical_Layer_Spec.pdf),
tables4-32–4-35 and5-4/5-16. The pinned DSView inventory was consulted only for
scope; no GPL implementation was copied or adapted.

Independent R1/R6/R7, wrong-index and complete CID/CSD vectors are authored in
`tests/dsl_sd_native.rs`; scheduling and compiled/interpreted checks retain exact
provenance. Their final execution belongs to the integrated verification pass.
