# AUD branch-address trace

`aud` consumes AUDCK, nAUDSYNC and four data lanes, sampling on rising AUDCK.
This is the branch destination address profile: a sync-high command nibble 8,
9, A or B selects 1, 2, 4 or 8 following sync-low data nibbles, least significant
nibble first. Short destinations replace low bits of the previous address;
a full eight-nibble address establishes the missing high bits. `initial_address`
is optional and defaults to unknown. Idle command 3 preserves context; other
unsupported commands invalidate it.

Each nibble streams immediately. At the next sync-high boundary, exactly the
advertised nibble count emits a destination (unknown if its high bits are
unknown); short/overlong packets end explicitly and invalidate context. A gap
or EOF aborts an unfinished packet. No more than nine nibble observations are
retained/counted per packet, and no packet-sized buffer exists. A single initial
provenance is held until the boundary establishing completeness.

This limited profile does not decode RAM-monitor mode, reset/mode pins,
device-specific idle timing or other trace packet types. It does not claim a
complete Renesas debug-port implementation.

Wire facts were cross-checked against the [Renesas AUD emulator documentation](https://www.renesas.com/en/document/mat/superh-family-e10a-usb-emulator-additional-document-users-manual-supplementary-information-using-1)
and the [libsigrokdecode AUD reference](https://github.com/sigrokproject/libsigrokdecode/tree/master/decoders/aud),
fenugrec (2016) and DreamSourceLab (2019), GPL-2.0-or-later. This TypeScript parser,
state representation, recovery rules and independent waveform vectors are
original; no GPL implementation body was translated or copied.
