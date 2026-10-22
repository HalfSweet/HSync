# Wii Nunchuk

Address `0x52` is configurable. Six-byte reports at offset zero expose
stick values, 10-bit accelerometer codes and active-low C/Z buttons. Acceleration
is raw, without factory calibration. Each missing report component prevents its
combined axis field; a STOP clears unfinished axes. The pointer is selected by
observed writes, and does not automatically wrap every six bytes.

`encoding` is explicitly `plain` or `legacy-xor`; legacy bytes use the published
XOR/add transformation. The F0=55, FB=00 initialization sequence is observable as
raw writes. Encryption negotiation, extension IDs, calibration and Wii MotionPlus
pass-through are outside this report profile.

Source: [WiiBrew Nunchuck extension documentation](https://wiibrew.org/wiki/Wiimote/Extension_Controllers/Nunchuck), original reverse-engineering notes; [libsigrokdecode Nunchuk reference](https://github.com/sigrokproject/libsigrokdecode/tree/master/decoders/nunchuk) was consulted for protocol facts. The TypeScript implementation and trace vectors are independently authored, with no copied GPL body.

Input is the existing typed `i2c.events@1` contract. Raw bytes are emitted immediately; all retained state has a fixed scalar bound, and emitted events use the bytes that establish their value as provenance. Unknown values are preserved rather than silently mapped to a known command.
