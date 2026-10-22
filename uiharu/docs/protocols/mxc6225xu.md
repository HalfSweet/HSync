# MXC6225XU

The default address is `0x15`. The limited reference profile names X/Y
signed eight-bit observations, orientation/shake status and detection settings.
Axis values are signed magnitudes in raw codes, without calibration or physical
acceleration conversion. Unknown registers remain raw. Register pointers are
observed from acknowledged writes; no initial pointer is assumed.

The original manufacturer PDF mirror is no longer reliably available. Register
facts were cross-checked against the MEMSIC MXC6225XU 2010 data-sheet index and
the upstream reference decoder. This is a limited reference profile, not a claim
of complete revision coverage.

Source: MEMSIC MXC6225XU 2010 data sheet (historical mirror `hands.com/~lkcl/eoma/kde_tablet/MXC6225XU.pdf`); [libsigrokdecode MXC6225XU reference](https://github.com/sigrokproject/libsigrokdecode/tree/master/decoders/mxc6225xu). Reference protocol facts were consulted; this bounded TypeScript state machine is independently authored, not a translation of the GPL implementation.

Input is the existing typed `i2c.events@1` contract. Raw bytes are emitted immediately; all retained state has a fixed scalar bound, and emitted events use the bytes that establish their value as provenance. Unknown values are preserved rather than silently mapped to a known command.
