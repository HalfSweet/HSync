# LTC2607 / LTC2617 / LTC2627

The default address is `0x10` (all address pins grounded); configure the
actual device address. Writes to global address `0x73` are also recognized.
The first byte is a command/channel, followed by an MSB-first two-byte data word.
The model selects 16-, 14- or 12-bit DAC codes. Named commands cover input write,
update, write/update, power-down and no-operation; reserved commands/channels are
marked invalid. Fields settle after the complete three-byte sequence. Extra
bytes remain raw; the chip does not acknowledge them. `reference_microvolts` defaults to 1500000 (range 1–5500000). The output-microvolts field is the input code’s ideal voltage, truncated to integer microvolts using code × reference / 2^resolution. Lower unused bits must be zero; otherwise code and voltage fields are invalid. Current latched output, reference selection and DAC state are not inferred. Reads do not produce DAC semantics.

Source: [Analog Devices LTC2607/LTC2617/LTC2627 data sheet](https://www.analog.com/media/en/technical-documentation/data-sheets/26071727fa.pdf), serial interface, figures 3–4 and table 2.

Input is the existing typed `i2c.events@1` contract. Raw bytes are emitted immediately; all retained state has a fixed scalar bound, and emitted events use the bytes that establish their value as provenance. Unknown values are preserved rather than silently mapped to a known command.
