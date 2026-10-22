# Modbus application protocol

A TypeScript DSL stream decoder over `modbus.events` v1. Implemented independently from the [Modbus Organization's MODBUS Application Protocol Specification V1.1b3](https://modbus.org/docs/Modbus_Application_Protocol_V1_1b3.pdf), §6 (function PDUs) and §7 (exceptions). The RTU framing and CRC remain the responsibility of `modbus-rtu`.

## Configuration and placement

Stack `uart → modbus-rtu → modbus-pdu`. Set `direction` to `request` for client traffic or `response` for server traffic. It cannot be inferred reliably from a single bidirectional receive line: several function request/response formats are identical. Use separate instances for separately captured client and server streams.

The input contract is declared in `dsl/contracts/modbus.ts`. Its JSON shape and schema version match the existing RTU stream. The RTU producer emits the generated contract types, which lets compiled and interpreted PDU decoders read the same stream.

## Stream: `modbus-pdu.events` v1

| Event      | Fields                                                 | Meaning                                                                                                                        |
| ---------- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
| `begin`    | `frame`, `address`, `function`, `direction`, `partial` | Observed RTU frame header, including the original function code's exception bit                                                |
| `field`    | `frame`, `field`, `value`                              | A complete function field: address, quantity, masks, subfunction, status, counters, exception code or byte count               |
| `register` | `frame`, `index`, `value`                              | One 16-bit big-endian register in the frame's register payload; the index starts at zero and is not an inferred memory address |
| `bits`     | `frame`, `offset`, `value`                             | Eight observed coil/discrete-input bits, low bit first; offset counts bits, with padding left visible                          |
| `data`     | `frame`, `offset`, `value`                             | A byte without a narrower interpretation, including device-specific additional data                                            |
| `end`      | `frame`, `reason`, `lengthOk`, `supported`, `valid`    | Transport outcome and application-structure checks                                                                             |

Fields and register data are emitted as soon as complete and remain provisional until `end`. `valid` requires a supported function, accepted structural length/range checks, a matching RTU CRC, a nonpartial header and a `complete` transport end. It never asserts that the requested operation succeeded, or that a particular response belongs to a particular request. `reason` preserves the lower RTU reason. Unsupported functions retain raw data with `supported = false`, never a guessed layout.

## Implemented layouts

| Function   | Request                                                                 | Response                                                            |
| ---------- | ----------------------------------------------------------------------- | ------------------------------------------------------------------- |
| 1, 2       | Starting address and quantity                                           | Byte count and packed coil/input bits                               |
| 3, 4       | Starting address and quantity                                           | Byte count and 16-bit register values                               |
| 5, 6       | Address and value                                                       | Echoed address and value                                            |
| 7          | No PDU data                                                             | Exception-status byte                                               |
| 8          | Subfunction and first data word                                         | Subfunction and first data word; additional bytes remain raw        |
| 11         | No PDU data                                                             | Status and event count                                              |
| 12         | No PDU data                                                             | Byte count, status, event/message counts and raw event bytes        |
| 15         | Address, quantity, byte count and coil bytes                            | Echoed address and quantity                                         |
| 16         | Address, quantity, byte count and registers                             | Echoed address and quantity                                         |
| 17         | No PDU data                                                             | Byte count, server ID, run indicator and additional raw data        |
| 20         | Bounded read-file descriptors (reference type, file, record, length)    | Streamed subresponse lengths, reference types and registers         |
| 21         | Write-file descriptors and streamed registers                           | Echoed descriptors and registers                                    |
| 24         | FIFO pointer address                                                    | Two-byte byte count, FIFO count and up to 31 registers              |
| 43 / 14    | MEI type, read-device-ID code and object ID                             | Conformity, continuation metadata and length-delimited object bytes |
| 22         | Address, AND mask and OR mask                                           | Echoed address and masks                                            |
| 23         | Read address/quantity, write address/quantity, byte count and registers | Byte count and read register values                                 |
| Exceptions | No inferred request layout                                              | Original exception function and one exception-code byte             |

## Streaming and recovery

Only one high byte and its provenance, fixed counters and the active header are retained, including one descriptor/object
subrecord cursor. File records, FIFO data and device-ID objects never require a
whole-frame or whole-object buffer. No PDU or complete RTU frame is buffered. Payloads exceeding the standard's 252 bytes after the function code are marked malformed. Declared byte counts, incomplete words, function-specific fixed lengths, request quantity limits and the special function-5 coil value are checked.

An assembled word spans its two contributing lower events, settling with the second. A word crossing the RTU's 32-byte batches holds progress at its first contributing event. Other fields reuse the lower event's provenance; their resolution is the lower transport batch's resolution. End events use the transport end's provenance. A gap or capture end closes an open PDU as aborted and discards pending state before the gap/end is forwarded.

## Limits

Client/server direction is configured, not auto-correlated. There is no request/response pairing, register map, floating-point interpretation or inference of requested addresses from responses. Diagnostic subfunction-specific counters and communication-event bit meanings are not fully interpreted; extra diagnostic words remain raw bytes. File reference types, file/record ranges, register counts, FIFO byte/count agreement
and MEI 14 object count/length agreement are checked incrementally. Other MEI
types retain raw bytes with `supported = false`; CANopen MEI 13 is not interpreted. Device-specific data remains raw. The schema marks structural validity separately from transport facts so neither malformed bytes nor an unimplemented function imply a successful operation.
