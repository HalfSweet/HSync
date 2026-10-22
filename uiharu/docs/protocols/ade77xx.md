<!-- SPDX-License-Identifier: Apache-2.0 -->

# ADE7753 and ADE7758

`ade77xx` selects explicit `model=ade7753|ade7758` (default ADE7758). The models have different address widths and register maps: ADE7758 uses seven address bits, 16-bit AWATTHR/BWATTHR/CWATTHR, 24-bit AIRMS and WFORM, and OPMODE at13h. All ADE7758 table17 register identifiers, lengths and signed/unsigned encodings are represented, including calibration addresses above3Fh and CHKSUM/Version7Eh/7Fh. ADE7753 retains its six-bit map and MODE/GAIN interpretation.

The complete SPI-event stream permits CS-less, intrinsically length-delimited command/register transfers. `framed=false` assumes the first observed complete byte is a communications-register command; `framed=true` requires CS and resets on each clean assertion. A gap, missing data direction or unknown-width command suspends alignment until a clean CS assertion; CS-less operation needs a new aligned capture. No entire transaction is buffered. Register values carry the span of all their observed bytes.

References: [Analog Devices ADE7758 RevE](https://www.analog.com/media/en/technical-documentation/data-sheets/ADE7758.pdf), table17 and serial interface; [ADE7753](https://www.analog.com/media/en/technical-documentation/data-sheets/ade7753.pdf). Meter calibration, physical energy units, analog behavior and complete mode-dependent measurement processing are outside this digital profile. The ADE7758 test uses SPI mode1, consistent with falling-edge register input/sampling; this overlay consumes decoded byte events rather than inferring electrical timing.

Two CS-less compiled/interpreted tests pass, covering the distinct widths/signed values, high addresses, OPMODE, scheduling and a gap inside a register. The earlier ADE7753 vectors remain in the SPI-device suite.

An initial SPI `begin` with `partial: true` is explicitly unaligned, including
continuous mode. No pixel, command or register interpretation is emitted from
that already-selected partial frame. A later clean CS assertion restores a known
boundary. Without CS, no partial-begin event exists: the documented initial-byte
alignment assumption remains available, while a subsequent gap still suspends
decoding. Dedicated stream regressions assert the partial-begin source event,
reject its semantic output, and check clean-CS recovery with generated/interpreted
parity, bounded queues and varying sample blocks.
