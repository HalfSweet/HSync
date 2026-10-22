<!-- SPDX-License-Identifier: Apache-2.0 -->

# Clocked RGB triplets

`rgb-led-spi` consumes the complete SPI event stream with eight-bit MSB-first MOSI words in modes0/3 (rising-edge sampling). CS and MISO may be absent. `framed=false` groups continuous triplets from the first complete byte and ignores normal CS framing; `framed=true` restarts on each clean CS assertion and rejects partial triplets at release. All six channel orders are explicit configuration choices.

A gap discards partial pixels and suspends alignment. An observed clean CS assertion can establish a new start; a CS-less capture requires a fresh aligned capture after lost samples. Missing MOSI aborts rather than inventing bytes. State retains only two byte values and their first provenance; a pixel span joins its three observed bytes.

This is a generic digital RGB profile. It does not infer chipset-specific reset/latch timing or APA102 start/end/brightness framing. Independent CS-less mode0/3 vectors, graph mode rejection, gaps and compiled/interpreted scheduling checks are in `tests/dsl_rgb_stream.rs`; the earlier six-order framed vectors remain in `dsl_spi_devices.rs`.

An initial SPI `begin` with `partial: true` is explicitly unaligned, including
continuous mode. No pixel, command or register interpretation is emitted from
that already-selected partial frame. A later clean CS assertion restores a known
boundary. Without CS, no partial-begin event exists: the documented initial-byte
alignment assumption remains available, while a subsequent gap still suspends
decoding. Dedicated stream regressions assert the partial-begin source event,
reject its semantic output, and check clean-CS recovery with generated/interpreted
parity, bounded queues and varying sample blocks.
