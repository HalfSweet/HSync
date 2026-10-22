<!-- SPDX-License-Identifier: Apache-2.0 -->

# XFP management memory

Address `0x50` and initial table 1 are configurable assumptions. This original
TypeScript implementation uses [XFP MSA revision 4.5 / INF-8077i](https://members.snia.org/document/dl/26514),
sections 4–5 and tables 31–59. The pinned libsigrokdecode XFP handler inventory
(Bert Vermeulen, GPL-3.0-or-later) was consulted to establish baseline scope;
no implementation body was copied or translated.

Lower memory includes identifier, signal conditioner data rate/loopbacks,
VPS voltage settings, named BER/wavelength/FEC control bytes, interrupt flags and
masks, general control/status, and diagnostic pairs. Alarm/warning thresholds
name their channel and high/low alarm/warning level. Temperature values and
thresholds retain exact signed 1/256 °C numerators; bias uses exact 2 µA codes,
TX/RX power exact 0.1 µW codes, and auxiliary inputs raw 16-bit codes. Auxiliary
monitor type events identify their declared quantities without guessing an
unobserved auxiliary configuration. A single unfinished big-endian pair is held.

Page-one upper memory includes extended identifier power class/CDR/refclock/CLEI,
connector descriptions, all defined transceiver capability bits, serial encoding,
minimum/maximum bit rate, all five link media, transmitter technology and its
control/cooling/detector/tunability flags, CDR/loopback support, vendor identity
bytes/revision, wavelength/tolerance, case temperature, supply power/current,
manufacture date, diagnostic mode, enhanced options and auxiliary types.
Wavelength is integer picometers, power integer milliwatts and supply current
integer milliamps. Text streams as individual bytes. Link events distinguish
exact distances, byte-255 strict lower bounds, and byte-zero unspecified lengths;
the raw code is always available. Date pairs require ASCII decimal digits and
basic month/day ranges, without claiming full calendar validation.

Table selection uses byte 127. Unsupported table writes may revert to table one,
so writes other than one invalidate the assumed table until readback. Gaps also
invalidate it. Unknown tables remain raw. Split reads discard unfinished pairs;
a later low byte never inherits a prior segment's high byte. Every byte streams
immediately, and aggregate events span their actual source pair. No whole
transaction, vendor string or memory image is buffered.

Optional PEC must be disabled for this profile. Management command execution,
vendor-table semantics, software password effects and inferred alarm occurrence
are outside its scope. BER/wavelength/FEC/interrupt fields retain their named raw
register values where the baseline provides no further interpretation. This is
wire observation, not proof that a requested hardware setting took effect.

`uiharu-interp/tests/i2c_devices.rs` includes independent threshold/control,
page-one capability/link sentinel, standard wavelength example, supply/date,
fragmented-pair and unknown-table vectors, with compiled/interpreted equality,
gaps, bounded queues, reader pauses and block/budget invariance.
