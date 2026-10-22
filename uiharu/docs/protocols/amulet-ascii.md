<!-- SPDX-License-Identifier: Apache-2.0 -->

# Amulet ASCII

This UART overlay implements the default ASCII command map: byte/word
get/set, strings and labels, RPC IDs/buffer, byte/word arrays, color values,
ACK/NACK and binary A0 page-jump checksum. Numeric fields use ASCII hexadecimal;
strings stream unchanged until NUL. Scalar replies are independently decoded,
without assuming the peer's request. No message or string is buffered.

Graphics line/rectangle/filled-rectangle commands (`D9`–`DB`) and echoed replies
(`E9`–`EB`) decode four 16-bit hexadecimal coordinates, RGB byte components and
one hexadecimal line-weight digit. Rectangle coordinates use origin plus deltas;
line coordinates use two endpoints. Filled rectangles report the transmitted
weight as unused. All 23 parameter characters must be observed before completion;
malformed hexadecimal remains invalid through the fixed message boundary.
Undocumented ASCII pixel opcodes, customized opcode maps, CRC-mode packets and
software flow control are outside this profile. Unsupported opcodes receive an explicit
unsupported end. Initial capture may begin mid-message; bytes remain observations
until a recognizable default opcode. `max_bytes` bounds a message's scalar counter
(default 4096, range 16–1048576); exceeding it invalidates that message. UART
framing/parity errors, breaks, gaps and EOF abort the current message.

Source: [Amulet Technologies GEMstudio Pro User’s Guide (2018)](https://www.amulettechnologies.com/wp-content/uploads/2018/02/Amulet-Technologies-GEMStudio-Pro-Users-Guide.pdf), ASCII Commands, Graphic Primitives (pages 148–153) and Jump to specific page. Default opcode facts were also cross-checked against the attributed GPL libsigrokdecode opcode list (Vesa-Pekka Palmu, 2019); the streaming parser is independently authored.

Input is the existing typed `uart.events@1` contract. Raw bytes are emitted immediately; all retained state has a fixed scalar bound, and emitted events use the bytes that establish their value as provenance. Unknown values are preserved rather than silently mapped to a known command.

Numeric field provenance covers the command prefix through the byte establishing the field. A single start provenance is held until message end; `max_bytes` bounds that progress hold, while every byte is emitted immediately.

`uiharu-interp/tests/uart_devices.rs` includes all six command/reply graphics vectors,
coordinate/color/weight boundaries, malformed hexadecimal and truncation, as well
as block cuts, gaps, pauses, bounded queues and compiled/interpreted checks.
