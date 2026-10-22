<!-- SPDX-License-Identifier: Apache-2.0 -->

# SD SPI registers

`sd-registers` supplements the block-oriented SPI card decoder with CMD9 CSD, CMD10 CID, CMD58 OCR and accepted CMD55/ACMD51 SCR transfers. SCR always reads eight bytes independent of block size and validates the trailing CRC16. A missing/rejected application prefix, truncated data or gap never fabricates an SCR.

SCR events expose raw64 bits, structure, SD specification/security/bus-width codes, erased-data state, later-version/security/command flags and manufacturer data. They span all observed register and CRC bytes. CSDv1/v2 capacity, CID identification/date and OCR flags remain supported. Raw flags do not establish the active bus width.

References: SD Association Physical Layer Simplified v2.00, sections5/7; [v5.10 table5-17](https://www.sdcard.org/cms/wp-content/themes/sdcard-org/dl.php?f=Part1_Physical_Layer_Simplified_Specification_Ver5.10.pdf) for SCR extended fields. Independent vectors cover OCR/CSD capacity, accepted/plain ACMD51, eight-byte CRC faults/truncation and compiled/interpreted scheduling. Filesystems, SDUC/CSDv3 and complete newer command sets remain outside scope.

The P1 register audit additionally exposes CID `productName` as exactly five raw
bytes, and CSD timing/rate/command classes, alignment, read/write current codes,
capacity multiplier, erase/write-protect geometry, write-speed/length, partial
writes, format/copy/protection and register CRC7. Obsolete v1 fields are absent
(`null`) for CSDv2. `crcOk` continues to report the SPI data block's CRC16;
`registerCrcOk` and `registerEndOk` separately validate the internal 128-bit
register. Thus a valid block checksum cannot hide a bad register checksum.

The named field inventory follows the manufacturer-authored
[SD Physical Layer Simplified v2.00](https://chlazza.nfshost.com/docs/Simplified_Physical_Layer_Spec.pdf)
tables5-2/5-4/5-16. The pinned DSView inventory was consulted for scope only; no
GPL implementation was copied or adapted. `tests/dsl_sd_registers.rs` supplies
independent full-field CSDv1, high-capacity CSDv2, product-name and separate CRC
vectors through both generated Rust and the IR, with fragmented scheduling and
small bounded queues. Unsupported CSD structures remain explicitly unsupported;
newer SD families and electrical behavior are future extensions rather than
missing labels from this declared profile.
