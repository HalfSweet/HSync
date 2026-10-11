# Modbus RTU

The Modbus RTU decoder reads the frames of a Modbus serial line in RTU mode from the characters of a UART stream and emits them as `modbus.events`. It follows the MODBUS over Serial Line Specification and Implementation Guide V1.02 (§2.5.1, RTU transmission mode, and the CRC generation in its appendix) and the MODBUS Application Protocol Specification V1.1b3 (§4.1, the PDU; §7, exception responses).

RTU mode frames its messages by silences on the line, so the decoder works from the timing of the characters: it is stacked on the [UART decoder](uart.md), whose stream must carry 8-bit characters.

```rust
let uart = graph.add(Uart::new(UartConfig {
    line: 0,
    baud: 19_200,
    parity: Parity::Even,
    ..UartConfig::default()
})?);
let modbus = graph.add(ModbusRtu::new());
graph.connect(logic.samples, Uart::input(uart))?;
graph.connect(Uart::events(uart), ModbusRtu::input(modbus))?;
let frames = graph.expose("modbus", ModbusRtu::events(modbus))?;
```

On a two-wire line requests and responses share one UART stream; separate receive and transmit lines take a UART and a Modbus decoder each.

## Requirements

The input port `uart` requires of the UART stream:

| Property   | Requirement                                 |
| ---------- | ------------------------------------------- |
| `dataBits` | `= 8`                                       |
| `bitOrder` | `= "lsb-first"`                             |
| `baud`     | `>= 1`; the character time is taken from it |

Parity and stop bits are free: even parity is the specification's default, and no parity with one or two stop bits is common. A connection that does not meet the requirements is refused, for example `modbus-rtu#1.uart requires dataBits = 8 of uart#0.events, which declares 7`.

## The stream: `modbus.events` v1

Frames are numbered from 1. A frame is a `begin`, its `data` and `crc`, and an `end`, all carrying the frame's id.

| Kind    | Fields                                                 | Meaning                                                                                                                                                                                                                                                      |
| ------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `begin` | `frame`, `address`, `function`, `exception`, `partial` | The frame's first two bytes: the server address (0 for a broadcast) and the function code, `null` in a frame of one byte. `exception` says whether the function code has its top bit set; `partial`, whether earlier bytes of the frame may have gone unseen |
| `data`  | `frame`, `offset: u32`, `bytes`                        | Bytes between the function code and the CRC, in order, at most 32 per event; `offset` counts from the first byte after the function code                                                                                                                     |
| `crc`   | `frame`, `value: u16`, `ok`                            | The frame's last two bytes as the CRC, sent low byte first, and whether it is the CRC of the bytes before it                                                                                                                                                 |
| `end`   | `frame`, `reason`                                      | The frame ended, for one of the reasons below                                                                                                                                                                                                                |

| Reason     | When                                                                                                                 |
| ---------- | -------------------------------------------------------------------------------------------------------------------- |
| `complete` | the silence that ends a frame followed it, and its CRC matched                                                       |
| `bad-crc`  | the silence followed it, but its CRC did not match                                                                   |
| `short`    | the frame had fewer than four bytes, no room for an address, a function code and a CRC                               |
| `broken`   | a character came after a silence of t1.5 or more inside the frame, or a break interrupted it; it is reported in full |
| `aborted`  | a gap or the end of the samples came before the silence that ends the frame was seen                                 |

```json
{"kind":"begin","frame":1,"address":17,"function":3,"exception":false,"partial":false}
{"kind":"data","frame":1,"offset":0,"bytes":[0,107,0,3]}
{"kind":"crc","frame":1,"value":34678,"ok":true}
{"kind":"end","frame":1,"reason":"complete"}
```

| Event                   | Span                                                                                                         | Settled                                          |
| ----------------------- | ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------ |
| `begin`                 | the address and function code characters                                                                     | when the function code is settled                |
| `begin` (a single byte) | the address character                                                                                        | the frame's end                                  |
| `data` (a full batch)   | the characters of the batch                                                                                  | when the character two after its last is settled |
| `data` (the rest)       | the characters of the batch                                                                                  | the frame's end                                  |
| `crc`                   | the last two characters                                                                                      | the frame's end                                  |
| `end`                   | empty, at the end of the last character + t3.5, or at the gap's start or the end of the samples when aborted | that tick                                        |

The output port `events` declares `mode = "rtu"`.

## Timing

A character is 11 bits on the line (a start bit, 8 data bits, a parity bit or a second stop bit, a stop bit), so a character time is 11 bit times at the UART stream's `baud`. The specification's two silences:

| Baud rate   | t1.5, inter-character     | t3.5, inter-frame         |
| ----------- | ------------------------- | ------------------------- |
| up to 19200 | 1.5 characters: 16.5 bits | 3.5 characters: 38.5 bits |
| above 19200 | 750 µs                    | 1.75 ms                   |

Both are turned into ticks of the source's clock and rounded up, so comparing a silence of whole ticks with them is exact. The silence before a character runs from the end of the character before it, the end of its stop bits, to its start edge.

## Decoding rules

- A frame is a run of characters; a silence of t3.5 or more ends it, and the character after that silence begins the next frame.
- A character after a silence of t1.5 or more but less than t3.5 still belongs to the frame, which ends `broken`: a receiver declares such a frame incomplete. A break on the line inside a frame breaks it too, and the silences are then measured from the end of the break; a break after the frame's silence leaves it alone.
- A frame ends once its silence is known to have passed: when the next character arrives after it, or when progress of the UART stream reaches the end of the last character + t3.5. The `end` is placed at that computed tick whichever told the decoder, so the events do not depend on how the samples were cut, and a frame ends while the line is still silent rather than when the next one begins.
- A gap or the end of the samples before that silence was seen aborts the frame, at the gap's start or at the end; its held bytes are reported as data and no `crc` is given. A frame whose silence had passed before the gap or the end ends as usual.
- Which two bytes are the CRC is only known when the frame ends, so the decoder holds the last two back: data goes out in batches of at most 32 bytes, a batch as soon as it is full and two more bytes have arrived, the rest when the frame ends.
- The CRC is the specification's CRC-16: the polynomial 0x8005 processed least significant bit first (0xA001), initial value 0xFFFF, no final XOR, sent low byte first. It is reported for every frame of four bytes or more that was not aborted.
- A frame ends `broken` as above, otherwise `short` with fewer than four bytes, otherwise `bad-crc` or `complete`.
- A frame is `partial` when its first character started less than t3.5 after the samples began, taken to be tick 0 where a source counts from, or after the end of a gap: the silence that must precede a frame was not seen, so earlier bytes of it may be missing. Its first two bytes are reported as address and function code all the same; its CRC almost always fails.
- Characters are taken as the UART read them: their parity and framing errors are not judged here, the CRC covering the frame. The UART stream shows them.
- Progress is passed on, but never past a byte the decoder still holds unreported.

## Function codes

The decoder reports function codes and data as they are. For reading the stream, the public function codes of the application protocol:

| Code   | Function                                  | Code   | Function                            |
| ------ | ----------------------------------------- | ------ | ----------------------------------- |
| `0x01` | Read Coils                                | `0x0F` | Write Multiple Coils                |
| `0x02` | Read Discrete Inputs                      | `0x10` | Write Multiple Registers            |
| `0x03` | Read Holding Registers                    | `0x11` | Report Server ID (serial line only) |
| `0x04` | Read Input Registers                      | `0x14` | Read File Record                    |
| `0x05` | Write Single Coil                         | `0x15` | Write File Record                   |
| `0x06` | Write Single Register                     | `0x16` | Mask Write Register                 |
| `0x07` | Read Exception Status (serial line only)  | `0x17` | Read/Write Multiple Registers       |
| `0x08` | Diagnostics (serial line only)            | `0x18` | Read FIFO Queue                     |
| `0x0B` | Get Comm Event Counter (serial line only) | `0x2B` | Encapsulated Interface Transport    |
| `0x0C` | Get Comm Event Log (serial line only)     |        |                                     |

An exception response carries the request's function code plus `0x80` and one data byte, the exception code: `0x01` illegal function, `0x02` illegal data address, `0x03` illegal data value, `0x04` server device failure, `0x05` acknowledge, `0x06` server device busy, `0x08` memory parity error, `0x0A` gateway path unavailable, `0x0B` gateway target device failed to respond.

## Limits

- The PDU is not interpreted: no function codes, exception codes or register maps.
- The address is reported as received; the specification reserves 248 to 255, which is not checked.
- Frames are not cut at the 256 bytes the specification allows.
- The start of the samples is taken to be tick 0 when deciding whether the first frame is partial: a stacked decoder does not learn where its input stream begins.
