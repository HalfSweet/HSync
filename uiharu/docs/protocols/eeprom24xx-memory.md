# EEPROM24xx memory addresses

`eeprom24xx-memory` is a DSL overlay of `i2c.events@1`. It supplements the existing
native `eeprom24xx` command decoder without changing that node or its public API.
Every data byte carries an optional reconstructed memory address and its ACK;
unknown counters remain unknown. Writes are observations until an ordinary STOP
emits `writeRequested: true`: one or more data bytes were acknowledged and STOP was observed. This does not establish nonvolatile completion, write-protect state or survival of power loss. A repeated START after write data is incomplete.
No memory contents or write-buffer transaction is retained.

`chip` selects generic geometry or the explicit 24C02/04/08/16/32/64/128/256/512
and 34AA04 profiles. The profile defines capacity, page size, address-byte count
and control-address block bits. Generic page size must be a power of two dividing capacity, capacity must fit the configured address bits, and the base address must leave block bits clear. These names describe the Microchip-compatible
geometry, not every manufacturer/revision sharing a similar marketing name.
For `generic`, configure `capacity` (a power of two, 1–524288), `page_bytes`, `address_bytes` (1/2)
and `block_bits` (0–3, with either address-byte width). `address` is a seven-bit
0x50–0x57 address; block bits are masked when matching the selected device.
`initial_counter` is optional and defaults to unknown, as does `initial_bank`.

Acknowledged word-address bytes establish a counter. Small-device block bits
are reconstructed from the control address; sequential reads wrap at capacity,
and page writes wrap within their page. Rejected write bytes are marked invalid
and do not advance the counter. A read's final NACK is normal. Gaps discard the
counter and bank. Aborted or restart-terminated writes invalidate their counter.

For 34AA04, SBA0/SBA1 use seven-bit addresses 0x36/0x37 and require an acknowledged
control plus two unacknowledged dummy bytes and a normal end. RBA is a read of
0x36: control ACK means bank zero, NACK bank one; at least one dummy read byte
must follow. Its bank is global to devices observing these commands, matching
the part's disabled chip-select comparison for bank commands. Sequential reads
wrap within the selected 256-byte bank. Software write-protect command execution
is not implemented; rejected write data remains observable.

References: [Microchip 24LC04B](https://ww1.microchip.com/downloads/en/DeviceDoc/24AA04-24LC04B-24FC04-4K-I2C-Serial-EEPROM-20001708P.pdf),
[24LC256](https://ww1.microchip.com/downloads/en/DeviceDoc/20001203W.pdf),
and [34AA04 DS20005271C](https://ww1.microchip.com/downloads/aemDocuments/documents/MPD/ProductDocuments/DataSheets/34AA04-4Kbit-I2C-Serial-EEPROM-with-Software-Write-Protect-Data-Sheet-DS20005271.pdf).
Independent traces cover page/capacity/block wrapping, NACK, unknown/current
counter, bank selection/readback, truncation, gaps and generated/interpreted
scheduling equivalence.

The onsemi CAT24M01 17-bit profile uses `chip: generic`, capacity 131072, page_bytes 256, address_bytes 2 and block_bits 1. Its A16 bit comes from the control address; writes wrap within 256-byte pages and sequential reads wrap at 128 KiB. See [onsemi CAT24M01 data sheet](https://www.onsemi.com/pdf/datasheet/cat24m01-d.pdf). Other vendor geometries can use explicit generic configuration; unique-ID areas and vendor write-protection command sets are not inferred from geometry.
