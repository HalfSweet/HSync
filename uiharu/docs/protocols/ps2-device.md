<!-- SPDX-License-Identifier: Apache-2.0 -->

# PS/2 keyboard and mouse interpretation

`ps2-device` consumes `ps2.events@1` from the existing physical PS/2 decoder.
Choose `keyboard-set1`, `keyboard-set2` (default), `keyboard-set3`, `mouse`,
`mouse-wheel` or `mouse-explorer` explicitly; identification traffic is
reported rather than used to guess a device type. The original implementation
uses wire facts from [Microchip AVR313](https://www.microchip.com/content/dam/mchp/documents/OTH/ApplicationNotes/ApplicationNotes/doc1235.pdf),
[Holtek HT82K629B](https://www.holtek.com.tw/webapi/11842/HT82K629Bv110.pdf)
and [Infineon PS2D](https://www.infineon.com/assets/row/public/documents/30/396/infineon-ps2d-001-13681-software-module-datasheets-en.pdf).

The keyboard profiles emit scan codes with make/break and extended flags.
Set 1 uses the high bit for break codes and recognizes its six-byte Pause and
four-byte Print Screen sequences. Set 3 uses its F0 break prefix without
applying Set 2 E0/E1 special-key rules.
The Set 2 profile recognizes the eight-byte Pause sequence and complete Print Screen make and
break sequences without inventing Shift key actions. Scan codes identify keys;
no language layout, text composition or pressed-key state is inferred. An
observed request for a different scan set suspends the configured keyboard
profile until its set is observed again. Reset selects Set 2, so a configured
Set 1/3 profile stays suspended after reset until its set is selected.
Shortened modifier-dependent Print Screen forms are outside these profiles.

The mouse profile supports standard three-byte packets: three buttons, signed
nine-bit X/Y movement and separate overflow flags. Magnitude plus a negative
flag represents each delta, including -256. `mouse-wheel` retains the fourth signed eight-bit wheel byte;
`mouse-explorer` interprets its signed four-bit wheel delta and buttons 4/5,
rejecting nonzero reserved upper bits. A separate `wheel` event carries
wheel magnitude/sign and the full button mask. These profiles are explicit;
observed device IDs do not silently change the configured packet length. The first packet byte's bit 3
is a synchronization heuristic; it cannot guarantee recovery at an arbitrary
capture boundary. Packet data may equal ACK or BAT values, so those values are
not consumed as control replies without a preceding observed host command.

Physically acknowledged host commands are reported separately from device
protocol acknowledgments. Parameter commands, scan-set queries, identification,
reset/BAT, mouse status and Read Data replies have bounded reply contexts.
Unacknowledged host frames, parity/stop errors, malformed special sequences,
gaps and EOF clear retained context. Unknown commands are reported as raw
command/reply bytes; no unsupported command's side effects are inferred.
Each unfinished key sequence or mouse packet retains one provenance and a
fixed set of scalars, never a transaction buffer. Aggregate events span their
observed source frames. Command and reply events use their individual frame
provenance.

The physical contract's generated types have exactly the existing enum and
JSON representation; the physical engine and signal behavior remain unchanged.
`uiharu-interp/tests/ps2_device.rs` exercises independent keyboard, mouse and
host-command traces, malformed input, partial sequences and compiled/interpreted
scheduling comparisons. Existing physical PS/2 tests also verify the contract.

Extended mouse wire formats follow the original manufacturer documentation in
[Microsoft keyboard/mouse driver guidance](https://learn.microsoft.com/en-us/windows-hardware/drivers/hid/keyboard-and-mouse-hid-client-drivers).
Set 1/3 scan framing also follows the Holtek keyboard command/scan tables.
Independent vectors cover the three scan sets, wheel sign boundaries and
five-button masks, with compiled/interpreted and block/gap checks.
