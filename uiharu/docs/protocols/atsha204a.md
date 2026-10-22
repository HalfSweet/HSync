# ATSHA204A I²C packet profile

The default address is `0x64`. Word-address flags are Reset (0), Sleep
(1), Idle (2) and Command (3). Command flags are excluded from the packet CRC.
Each contiguous addressed command/response packet exposes count, named opcode,
parameters, immediate raw data and received/expected CRC16 (polynomial 0x8005,
initial zero, input bits least significant first). Responses of count four expose
a status byte. Count range, missing data and bytes beyond count are checked.
The count limit is 84, and retained state is fixed scalars plus one byte.

This profile does not assemble FIFO reads split across addressed I²C segments,
decode wake pulses, execute authentication, infer configuration zones, or correlate
response data to prior commands. Idle/Sleep/Reset flags are observations rather
than inferred device state. Packet fields are provisional until CRC/end results.

Source: [Microchip ATSHA204A DS40002025A](https://ww1.microchip.com/downloads/en/devicedoc/atsha204a-data-sheet-40002025a.pdf), sections 6 and 8.

Input is the existing typed `i2c.events@1` contract. Raw bytes are emitted immediately; all retained state has a fixed scalar bound, and emitted events use the bytes that establish their value as provenance. Unknown values are preserved rather than silently mapped to a known command.
