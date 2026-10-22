# AVR XMEGA PDI

`avr-pdi` samples PDI_DATA on rising PDI_CLK. Characters use one start bit,
eight LSB-first data bits, even parity and at least one high stop bit. Extra
high stop/idle clocks are accepted. Twelve successive sampled lows produce
BREAK; a second BREAK is also recognized. Every complete character reports its
parity and stop result independently of instruction interpretation.

Instruction alignment defaults to unknown until an observed BREAK. Set
`start_aligned` only when capture starts at a known instruction boundary. A
parity/framing error, reserved opcode or gap loses instruction and pointer
context; following characters remain raw until BREAK reestablishes it. This
prevents a trailing operand from becoming a fabricated instruction after loss.

The instruction layer supports LDS, LD, STS, ST, LDCS, STCS, KEY and REPEAT;
address/data widths, pointer mode and control-register number are explicit.
Operands stream immediately and multi-byte values assemble little-endian.
Direct and pointer addresses remain unknown until a full address establishes
their high bits; subsequent partial updates preserve only known high bits.
Post-increment uses the observed transfer width. The KEY's eight bytes are
emitted individually, without exposing a lossy 64-bit JSON key value.

REPEAT stores a 32-bit operand plus one in a 64-bit scalar counter, and is
consumed by the next instruction. Even a count of 2^32 is streamed one value at
a time, without allocation or a transaction-sized buffer. KEY and REPEAT do not
repeat themselves. EOF aborts an unfinished instruction. Progress is held only
for one unfinished character/value (at most eight operand bytes), not for the
entire repeated instruction.

The profile does not identify which peer electrically drives the half-duplex
line, detect contention, infer successful NVM programming or model device-specific
enable/disable timing. PDI-over-JTAG symbols are outside this physical profile.

References: [Microchip/Atmel XMEGA A Manual 8077I](https://ww1.microchip.com/downloads/en/DeviceDoc/doc8077.pdf),
sections 29.3–29.6 (including little-endian register updates and repeat-count
semantics), and [AVR1612](https://ww1.microchip.com/downloads/en/Appnotes/doc8282.pdf)
for the synchronous physical timing. AVR1612's illustrative byte-order examples
contain inconsistent labels; this profile follows the architecture manual's
register ordering. Opcode/width facts were cross-checked against the attributed
GPL libsigrokdecode AVR PDI reference (Uwe Hermann, Gerhard Sittig and
DreamSourceLab); the bounded TypeScript parser and vectors are independently
authored, not a translation of that implementation.
