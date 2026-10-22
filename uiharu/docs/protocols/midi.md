# MIDI

The MIDI decoder reads the messages of a MIDI 1.0 link from the characters of a UART stream and emits them as `midi.events`. It follows the MIDI 1.0 Detailed Specification, document version 4.2.1 (MIDI Manufacturers Association): the hardware, status and data bytes, running status, the messages and the data bytes each takes (Tables I to VII), system exclusive messages, and the receiver's running status buffer and the undefined status bytes of its Additional Explanations and Application Notes.

MIDI sends its bytes as UART characters at 31.25 kbaud, a start bit, eight data bits least significant bit first and a stop bit, so the decoder is stacked on the [UART decoder](uart.md):

```rust
let uart = graph.add(Uart::new(UartConfig {
    line: 0,
    baud: Midi::BAUD,
    ..UartConfig::default()
})?);
let midi = graph.add(Midi::new());
graph.connect(logic.samples, Uart::input(uart))?;
graph.connect(Uart::events(uart), Midi::input(midi))?;
let messages = graph.expose("midi", Midi::events(midi))?;
```

A MIDI cable carries messages in one direction; a device's MIDI In and MIDI Out take a UART and a MIDI decoder each. The decoder takes no configuration.

## Requirements

The input port `uart` requires of the UART stream:

| Property   | Requirement                          |
| ---------- | ------------------------------------ |
| `baud`     | `in 30938..=31562`, 31.25 kbaud ±1 % |
| `dataBits` | `= 8`                                |
| `parity`   | `= "none"`                           |
| `stopBits` | `= "one"`                            |
| `bitOrder` | `= "lsb-first"`                      |

A connection that does not meet them is refused, for example `midi#1.uart requires baud in 30938..=31562 of uart#0.events, which declares 38400`.

## The stream: `midi.events` v1

Messages are numbered from 1 in the order they begin, real-time messages aside. A channel or system common message is one event, sent with its last data byte. A system exclusive message, which may be long, is a run of `sysex` events carrying its data as it comes and a `sysex-end`, all carrying its id. Real-time messages are events of their own, wherever they fall.

| Kind              | Fields                                                      | Meaning                                                                                                                                                                                                                                                                                       |
| ----------------- | ----------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `channel-message` | `message`, `channel`, `status`, `data1`, `data2`, `running` | A channel voice or mode message: the channel, the low nibble of the status byte (0 to 15 for MIDI channels 1 to 16), the message from its high nibble, its data bytes as sent (`data2` is `null` for a message of one), and whether it was sent under running status, without its status byte |
| `system-common`   | `message`, `status`, `data1`, `data2`                       | A system common message and its data bytes as sent, `null` where it has none                                                                                                                                                                                                                  |
| `realtime`        | `status`, `byte`                                            | A system real-time message and its status byte                                                                                                                                                                                                                                                |
| `sysex`           | `message`, `offset: u32`, `bytes`                           | Data bytes of a system exclusive message, in order, at most 32 per event; `offset` counts from the byte after the start of exclusive                                                                                                                                                          |
| `sysex-end`       | `message`, `bytes: u32`, `reason`                           | The end of a system exclusive message and the number of data bytes it carried                                                                                                                                                                                                                 |
| `error`           | `reason`, `byte`                                            | Bytes that make no message, with the byte the error is about                                                                                                                                                                                                                                  |

| Status (`channel-message`) | Status byte | Data bytes                                                                     |
| -------------------------- | ----------- | ------------------------------------------------------------------------------ |
| `note-off`                 | 0x8n        | note number, velocity                                                          |
| `note-on`                  | 0x9n        | note number, velocity; a velocity of 0 is a note off                           |
| `poly-pressure`            | 0xAn        | note number, pressure                                                          |
| `control-change`           | 0xBn        | controller number, value; controllers 120 to 127 are the channel mode messages |
| `program-change`           | 0xCn        | program number                                                                 |
| `channel-pressure`         | 0xDn        | pressure                                                                       |
| `pitch-bend`               | 0xEn        | the low seven bits of the bend, then its high seven bits                       |

| Status (`system-common`)  | Status byte | Data bytes                                                      |
| ------------------------- | ----------- | --------------------------------------------------------------- |
| `time-code-quarter-frame` | 0xF1        | `0nnndddd`: the message type `nnn` and its value `dddd`         |
| `song-position-pointer`   | 0xF2        | the low seven bits of the position in MIDI beats, then its high |
| `song-select`             | 0xF3        | song number                                                     |
| `tune-request`            | 0xF6        | none                                                            |

| Status (`realtime`) | Status byte                                            |
| ------------------- | ------------------------------------------------------ |
| `clock`             | 0xF8, timing clock                                     |
| `start`             | 0xFA                                                   |
| `continue`          | 0xFB                                                   |
| `stop`              | 0xFC                                                   |
| `active-sensing`    | 0xFE                                                   |
| `reset`             | 0xFF, system reset                                     |
| `undefined`         | 0xF9 or 0xFD, which the specification leaves undefined |

| Reason (`sysex-end`) | When                                                             |
| -------------------- | ---------------------------------------------------------------- |
| `end-of-exclusive`   | an end of exclusive, 0xF7                                        |
| `interrupted`        | another status byte, other than real-time, which ends it as well |
| `aborted`            | a gap or the end of the samples came first                       |

| Reason (`error`)              | When                                                                                                                                               | `byte`                                           |
| ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------ |
| `unexpected-data`             | a data byte with no status in effect: a system message cleared the running status, or none was seen since the samples began or resumed after a gap | the data byte                                    |
| `incomplete`                  | a status byte, other than real-time, came before a message's last data byte                                                                        | the message's status byte, or its running status |
| `aborted`                     | a gap or the end of the samples came before a message's last data byte                                                                             | the message's status byte, or its running status |
| `undefined-status`            | 0xF4 or 0xF5, the undefined system common status bytes                                                                                             | the status byte                                  |
| `unexpected-end-of-exclusive` | an end of exclusive with no system exclusive message to end                                                                                        | 0xF7                                             |

```json
{"kind":"channel-message","message":1,"channel":0,"status":"note-on","data1":60,"data2":100,"running":false}
{"kind":"realtime","status":"clock","byte":248}
{"kind":"system-common","message":2,"status":"song-position-pointer","data1":16,"data2":2}
{"kind":"sysex","message":3,"offset":0,"bytes":[67,16,76]}
{"kind":"sysex-end","message":3,"bytes":3,"reason":"end-of-exclusive"}
{"kind":"error","reason":"unexpected-data","byte":62}
```

| Event                                  | Span                                                                                                                            | Settled                                                                  |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------ |
| `channel-message`, `system-common`     | the status byte, or the first data byte under running status, … the last data byte                                              | the last data byte's stop bit                                            |
| `realtime`, `error` (but those below)  | the byte                                                                                                                        | its stop bit                                                             |
| `sysex` (a full batch)                 | its bytes, from the start of exclusive for the first batch                                                                      | the last byte's stop bit                                                 |
| `sysex` (the rest)                     | its bytes, from the start of exclusive for the first batch                                                                      | the stop bit of the status byte that ends it, the gap's start or the end |
| `sysex-end` (`end-of-exclusive`)       | the end of exclusive, and the start of exclusive when no batch covered it                                                       | the end of exclusive's stop bit                                          |
| `sysex-end` (`interrupted`, `aborted`) | empty, at the start of the status byte, the gap or the end of the samples; from the start of exclusive when no batch covered it | the status byte's stop bit, the gap's start or the end                   |
| `error` (`incomplete`, `aborted`)      | the message's bytes so far                                                                                                      | the stop bit of the status byte that cut it, the gap's start or the end  |

The output port `events` declares no properties.

Data bytes are reported as sent. Two make a 14-bit value, low seven bits first: a pitch bend is `data2 × 128 + data1`, 0 to 16383 with 8192 in the middle, and a song position `data2 × 128 + data1` MIDI beats of six clocks each. The channel mode messages are control changes 120 (all sound off), 121 (reset all controllers), 122 (local control), 123 (all notes off), 124 (omni mode off), 125 (omni mode on), 126 (mono mode on) and 127 (poly mode on).

## Decoding rules

- A byte with bit 7 set is a status byte, any other a data byte.
- A real-time status byte, 0xF8 to 0xFF, is reported at once wherever it comes, even between the bytes of another message or inside a system exclusive message, and changes nothing else: the message it falls in goes on, and the running status stays.
- Any other status byte ends what is under way: a message still missing data bytes is `error { incomplete }`, a system exclusive message ends `interrupted` — or `end-of-exclusive`, when the byte is 0xF7.
- A channel status byte begins a message of one data byte (program change, channel pressure) or two (the others) and becomes the running status. System common and exclusive status bytes, 0xF0 to 0xF7, clear it — the undefined 0xF4 and 0xF5 too.
- A data byte when no message is under way begins a message under the running status. Without a running status it is `error { unexpected-data }`; since the running status is not known at the start of the samples or after a gap, data bytes before the first status byte there are reported so as well.
- System exclusive data goes out in batches of at most 32 bytes, each as soon as it is full and the rest when the message ends.
- A gap or the end of the samples ends what is under way: a message still missing data bytes is `error { aborted }`, a system exclusive message ends `aborted`. A gap clears the running status.
- Characters are taken as the UART read them: framing errors are not judged here, and breaks, which carry no byte, are passed over. The UART stream shows both.
- Progress is passed on, held back to the start of the message under way, or of the system exclusive bytes not reported yet.

## Limits

- Messages are not interpreted: neither note on with velocity 0, nor channel mode messages, MIDI time code, manufacturers' identifiers or universal system exclusive messages.
- The active sensing timeout is not checked.
- A system reset is a real-time message like the others: it does not clear the running status.

## Semantic consumer

The exact `midi.events@1` JSON contract is declared in `dsl/contracts/midi.ts`;
the native producer emits those generated types. Message IDs are scalar `u64`
values. Native status conversion/query helpers and the event's message lookup
are exported as `ChannelStatusExt`, `CommonStatusExt`, `RealtimeStatusExt` and
`MidiEventExt` traits.

Stack [midi-semantics](midi-semantics.md) above this transport for note/GM drum
and controller names, velocity-zero note-off, pitch bend, song position,
quarter-frame fields and bounded SysEx manufacturer identifiers. The lower
transport's raw message events remain available independently.
