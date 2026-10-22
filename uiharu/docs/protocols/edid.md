<!-- SPDX-License-Identifier: Apache-2.0 -->

# EDID over DDC

The default seven-bit EDID address is `0x50`; the E-DDC segment address is `0x30`.
`initial_segment` defaults to an explicit assumption of zero. Set it to null when
that initial context is unknown. Acknowledged segment writes select segments
0–127. NACK and normal STOP reset the segment to zero, as E-DDC requires; gaps
make it unknown. Repeated START preserves a selected segment. The eight-bit
word offset wraps within its current 256-byte segment. Devices that wrap earlier
require separate explicitly addressed reads. Raw byte events retain the offset
and an optional absolute byte address; decoded fields identify their EDID block.

The base profile decodes header/manufacturer/product/serial identity,
manufacture/version/display fields, ten-bit chromaticity numerators (denominator
1024), all 17 established timing flags, standard timing dimensions/aspect/refresh,
and complete detailed timing fields. Detailed timing includes pixel clock,
active/blanking, sync offset/pulse widths, physical image size, borders,
interlace/stereo and sync format flags. Monitor descriptors stream named
name/serial/text bytes; range descriptors expose vertical/horizontal limits,
revision-1.4 offsets, maximum pixel clock and additional-timing support code.
Additional color-point and additional-standard-timing descriptor types stream
named bytes, matching the pinned reference’s section-label scope. GTF/CVT
coefficient expansion and other optional monitor descriptors remain raw bytes.

CTA extension revisions 1–3 expose header fields and detailed timing slots.
Revision 3 additionally walks the bounded data-block collection: tag/length,
short video codes/native flags, audio format/channel/rate/detail fields,
vendor IEEE OUI, speaker mask and extended tag. Vendor payload and unknown
extended-tag payload remain raw bytes. The decoder does not expand every CTA
video code into a mode table or interpret HDMI/vendor-specific payloads.
A malformed block crossing the DTD boundary stops semantic collection parsing;
its raw bytes remain available. DisplayID and other extension formats expose
their tag and raw bytes rather than receiving CTA semantics.

Raw bytes stream immediately; descriptor text and payloads are never buffered as
whole arrays. Fixed scalar state retains only pending semantic values. A
contiguous 128-byte block beginning at a block boundary receives a modulo-256
checksum result and retains at most that block's provenance. Split reads retain
the observed offset but discard incomplete semantic values and checksum state;
they never fabricate a combined checksum or reuse a previous descriptor's bits.
Events span the bytes that establish their value, including low chromaticity bits
and descriptor/block headers where required.

The original TypeScript implementation uses wire facts from:

- [VESA Enhanced EDID Release A, revision 2](https://glenwing.github.io/docs/VESA-EEDID-A2.pdf), base block and descriptor tables.
- [VESA E-DDC 1.2](https://glenwing.github.io/docs/VESA-EDDC-1.2.pdf), sections 2.2.5–2.2.6 and 6.
- [CEA-861-D](https://pdfcoffee.com/cea-standard-pdf-free.html), primary-authored standard mirrored as text, section 7.5 and tables 28–37.

`uiharu-interp/tests/i2c_devices.rs` covers identity/checksum, split semantic
fields, chromaticity, established/full detailed timing and range vectors,
segment reset/absolute addresses, CTA audio/video/vendor/speaker/extended blocks,
malformed collection bounds, gaps, progress, block cuts and compiled/interpreted
equality.

The pinned libsigrokdecode EDID inventory (Bert Vermeulen, GPL-3.0-or-later)
was consulted only to determine reference scope. No implementation body was
copied or translated; all decoding rules above use the primary standards.
