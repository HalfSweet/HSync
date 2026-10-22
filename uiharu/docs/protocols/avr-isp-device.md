<!-- SPDX-License-Identifier: Apache-2.0 -->

# AVR ISP signature observations

`avr-isp-device` consumes the canonical `avrisp.events@1` transport from
`avr-isp`. Its TypeScript contract preserves complete `bytes: [u8;4]` arrays,
partial-byte vectors, optional fields and the existing JSON schema exactly.
The native transport remains the producer; all new interpretation is TypeScript.

`signature-byte` reports each observed Read Signature Byte address and answer.
Consecutive addresses0,1,2 in the same observed SPI transaction produce a
`signature` carrying the raw24-bit value, manufacturer/family/part bytes and
vendor/device classifications. A new address0 starts a new candidate; another
command, unknown/error event, out-of-order address, changed transaction, gap or
end discards it. The aggregate spans the three instructions and settles when
the last answer arrives; individual bytes are emitted immediately.

Manufacturer `0x1E` is Atmel/Microchip. The independently verified initial
name table contains ATmega328 (`0x1E9514`), ATmega328P (`0x1E950F`) and
ATmega328PB (`0x1E9516`), from [Microchip AT15007 §3.6](https://onlinedocs.microchip.com/oxy/GUID-CBDC1838-0100-4F26-A45A-134958193C3B-en-US-4/GUID-5A8FB4BC-BC6B-47F2-AACE-6B6B107D5E8A.html).
All other signatures remain numeric with `device: unknown`; there is no
unverified device-name table. `sync-warning` reports Programming Enable's
explicitly observed false echo field. Neither event implies successful
programming or authenticates a physical device.

The current native instruction event exposes MISO only as `dataOut` during
byte4 and `inSync` for Programming Enable byte3. Additional upstream warnings
check MISO during bytes2/3 of other instructions; these bytes are absent from
the contract. Such warnings cannot be implemented faithfully by this consumer.
A future lower contract version could include `reply: readonly [u8,u8,u8,u8]`
with the four observed MISO bytes; unknown/missing replies must stay optional.
This is a transport information gap, not a DSL fixed-array blocker. The current
schema is preserved rather than inventing absent replies or copying GPL code.

The native `avr_isp::OpId` is now a `u64` alias. `Command::ALL/from_bytes`
are supplied by the reexported `CommandExt` trait. These mechanical source API
changes establish canonical generated DTO ownership without changing wire
values or the native grouping/command-decoding behavior. Existing AVR tests
remain the transport acceptance vectors; the new completion suite checks
native-to-DSL type identity, known/unknown signatures, echo warnings,
compiled/interpreted equivalence and block/queue invariance.
