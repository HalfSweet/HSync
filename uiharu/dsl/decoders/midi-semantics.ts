// SPDX-License-Identifier: Apache-2.0
/** Independent MIDI 1.0 semantic fields from MIDI Association public message,
 * controller, General MIDI percussion and manufacturer tables. No reference source copied.
 * https://midi.org/summary-of-midi-1-0-messages
 * https://midi.org/midi-1-0-control-change-messages
 * https://midi.org/general-midi
 * https://midi.org/SysExIDtable
 */
import type { Range, bool, u8, u16, u32, u64 } from 'uiharu'
import { StreamDecoder, defineStreamDecoder, eventSchema } from 'uiharu'
import { midiEvents, type MidiEvent } from '../contracts/midi'
export interface Config {
  percussionChannel: u8
}
export type PitchClass =
  | 'c'
  | 'c-sharp'
  | 'd'
  | 'd-sharp'
  | 'e'
  | 'f'
  | 'f-sharp'
  | 'g'
  | 'g-sharp'
  | 'a'
  | 'a-sharp'
  | 'b'
export type Controller =
  | 'bank-select'
  | 'modulation-wheel'
  | 'breath-controller'
  | 'foot-controller'
  | 'portamento-time'
  | 'data-entry'
  | 'channel-volume'
  | 'balance'
  | 'pan'
  | 'expression'
  | 'effect-control-1'
  | 'effect-control-2'
  | 'general-purpose-1'
  | 'general-purpose-2'
  | 'general-purpose-3'
  | 'general-purpose-4'
  | 'damper-pedal'
  | 'portamento'
  | 'sostenuto'
  | 'soft-pedal'
  | 'legato-footswitch'
  | 'hold-2'
  | 'sound-variation'
  | 'timbre'
  | 'release-time'
  | 'attack-time'
  | 'brightness'
  | 'decay-time'
  | 'vibrato-rate'
  | 'vibrato-depth'
  | 'vibrato-delay'
  | 'sound-controller-10'
  | 'general-purpose-5'
  | 'general-purpose-6'
  | 'general-purpose-7'
  | 'general-purpose-8'
  | 'portamento-control'
  | 'high-resolution-velocity-prefix'
  | 'effects-1-depth'
  | 'effects-2-depth'
  | 'effects-3-depth'
  | 'effects-4-depth'
  | 'effects-5-depth'
  | 'data-increment'
  | 'data-decrement'
  | 'nrpn-lsb'
  | 'nrpn-msb'
  | 'rpn-lsb'
  | 'rpn-msb'
  | 'all-sound-off'
  | 'reset-all-controllers'
  | 'local-control'
  | 'all-notes-off'
  | 'omni-off'
  | 'omni-on'
  | 'mono-on'
  | 'poly-on'
export type Drum =
  | 'acoustic-bass-drum'
  | 'bass-drum-1'
  | 'side-stick'
  | 'acoustic-snare'
  | 'hand-clap'
  | 'electric-snare'
  | 'low-floor-tom'
  | 'closed-hi-hat'
  | 'high-floor-tom'
  | 'pedal-hi-hat'
  | 'low-tom'
  | 'open-hi-hat'
  | 'low-mid-tom'
  | 'hi-mid-tom'
  | 'crash-cymbal-1'
  | 'high-tom'
  | 'ride-cymbal-1'
  | 'chinese-cymbal'
  | 'ride-bell'
  | 'tambourine'
  | 'splash-cymbal'
  | 'cowbell'
  | 'crash-cymbal-2'
  | 'vibraslap'
  | 'ride-cymbal-2'
  | 'hi-bongo'
  | 'low-bongo'
  | 'mute-hi-conga'
  | 'open-hi-conga'
  | 'low-conga'
  | 'high-timbale'
  | 'low-timbale'
  | 'high-agogo'
  | 'low-agogo'
  | 'cabasa'
  | 'maracas'
  | 'short-whistle'
  | 'long-whistle'
  | 'short-guiro'
  | 'long-guiro'
  | 'claves'
  | 'hi-wood-block'
  | 'low-wood-block'
  | 'mute-cuica'
  | 'open-cuica'
  | 'mute-triangle'
  | 'open-triangle'
export type Manufacturer =
  | 'sequential'
  | 'moog'
  | 'kawai'
  | 'roland'
  | 'korg'
  | 'yamaha'
  | 'casio'
  | 'akai'
  | 'educational'
  | 'universal-non-realtime'
  | 'universal-realtime'
  | 'novation'
export type Action = 'on' | 'off' | 'pressure'
export type Event =
  | {
      kind: 'note'
      message: u64
      channel: u8
      note: u8
      pitchClass: PitchClass
      octave: u8
      belowZero: bool
      action: Action
      value: u8 | undefined
      drum: Drum | undefined
    }
  | {
      kind: 'controller'
      message: u64
      channel: u8
      number: u8
      controller: Controller | undefined
      lsb: bool
      value: u8 | undefined
    }
  | { kind: 'pitch-bend'; message: u64; channel: u8; value: u16; magnitude: u16; negative: bool }
  | { kind: 'song-position'; message: u64; midiBeats: u16; midiClocks: u32 }
  | { kind: 'quarter-frame'; message: u64; piece: u8; value: u8 }
  | {
      kind: 'manufacturer'
      message: u64
      identifier: u32
      extended: bool
      manufacturer: Manufacturer | undefined
    }
export const events = eventSchema<Event>('midi-semantics.events', 1).roles({
  note: 'data',
  controller: 'control',
  'pitch-bend': 'data',
  'song-position': 'control',
  'quarter-frame': 'control',
  manufacturer: 'address',
})
function pitchClass(note: u8): PitchClass {
  switch (note % 12) {
    case 0:
      return 'c'
    case 1:
      return 'c-sharp'
    case 2:
      return 'd'
    case 3:
      return 'd-sharp'
    case 4:
      return 'e'
    case 5:
      return 'f'
    case 6:
      return 'f-sharp'
    case 7:
      return 'g'
    case 8:
      return 'g-sharp'
    case 9:
      return 'a'
    case 10:
      return 'a-sharp'
    case 11:
      return 'b'
    default:
      return 'c'
  }
}
function controllerName(value: u8): Controller | undefined {
  switch (value) {
    case 0:
      return 'bank-select'
    case 1:
      return 'modulation-wheel'
    case 2:
      return 'breath-controller'
    case 4:
      return 'foot-controller'
    case 5:
      return 'portamento-time'
    case 6:
      return 'data-entry'
    case 7:
      return 'channel-volume'
    case 8:
      return 'balance'
    case 10:
      return 'pan'
    case 11:
      return 'expression'
    case 12:
      return 'effect-control-1'
    case 13:
      return 'effect-control-2'
    case 16:
      return 'general-purpose-1'
    case 17:
      return 'general-purpose-2'
    case 18:
      return 'general-purpose-3'
    case 19:
      return 'general-purpose-4'
    case 64:
      return 'damper-pedal'
    case 65:
      return 'portamento'
    case 66:
      return 'sostenuto'
    case 67:
      return 'soft-pedal'
    case 68:
      return 'legato-footswitch'
    case 69:
      return 'hold-2'
    case 70:
      return 'sound-variation'
    case 71:
      return 'timbre'
    case 72:
      return 'release-time'
    case 73:
      return 'attack-time'
    case 74:
      return 'brightness'
    case 75:
      return 'decay-time'
    case 76:
      return 'vibrato-rate'
    case 77:
      return 'vibrato-depth'
    case 78:
      return 'vibrato-delay'
    case 79:
      return 'sound-controller-10'
    case 80:
      return 'general-purpose-5'
    case 81:
      return 'general-purpose-6'
    case 82:
      return 'general-purpose-7'
    case 83:
      return 'general-purpose-8'
    case 84:
      return 'portamento-control'
    case 88:
      return 'high-resolution-velocity-prefix'
    case 91:
      return 'effects-1-depth'
    case 92:
      return 'effects-2-depth'
    case 93:
      return 'effects-3-depth'
    case 94:
      return 'effects-4-depth'
    case 95:
      return 'effects-5-depth'
    case 96:
      return 'data-increment'
    case 97:
      return 'data-decrement'
    case 98:
      return 'nrpn-lsb'
    case 99:
      return 'nrpn-msb'
    case 100:
      return 'rpn-lsb'
    case 101:
      return 'rpn-msb'
    case 120:
      return 'all-sound-off'
    case 121:
      return 'reset-all-controllers'
    case 122:
      return 'local-control'
    case 123:
      return 'all-notes-off'
    case 124:
      return 'omni-off'
    case 125:
      return 'omni-on'
    case 126:
      return 'mono-on'
    case 127:
      return 'poly-on'
    default:
      return undefined
  }
}
function drumName(value: u8): Drum | undefined {
  switch (value) {
    case 35:
      return 'acoustic-bass-drum'
    case 36:
      return 'bass-drum-1'
    case 37:
      return 'side-stick'
    case 38:
      return 'acoustic-snare'
    case 39:
      return 'hand-clap'
    case 40:
      return 'electric-snare'
    case 41:
      return 'low-floor-tom'
    case 42:
      return 'closed-hi-hat'
    case 43:
      return 'high-floor-tom'
    case 44:
      return 'pedal-hi-hat'
    case 45:
      return 'low-tom'
    case 46:
      return 'open-hi-hat'
    case 47:
      return 'low-mid-tom'
    case 48:
      return 'hi-mid-tom'
    case 49:
      return 'crash-cymbal-1'
    case 50:
      return 'high-tom'
    case 51:
      return 'ride-cymbal-1'
    case 52:
      return 'chinese-cymbal'
    case 53:
      return 'ride-bell'
    case 54:
      return 'tambourine'
    case 55:
      return 'splash-cymbal'
    case 56:
      return 'cowbell'
    case 57:
      return 'crash-cymbal-2'
    case 58:
      return 'vibraslap'
    case 59:
      return 'ride-cymbal-2'
    case 60:
      return 'hi-bongo'
    case 61:
      return 'low-bongo'
    case 62:
      return 'mute-hi-conga'
    case 63:
      return 'open-hi-conga'
    case 64:
      return 'low-conga'
    case 65:
      return 'high-timbale'
    case 66:
      return 'low-timbale'
    case 67:
      return 'high-agogo'
    case 68:
      return 'low-agogo'
    case 69:
      return 'cabasa'
    case 70:
      return 'maracas'
    case 71:
      return 'short-whistle'
    case 72:
      return 'long-whistle'
    case 73:
      return 'short-guiro'
    case 74:
      return 'long-guiro'
    case 75:
      return 'claves'
    case 76:
      return 'hi-wood-block'
    case 77:
      return 'low-wood-block'
    case 78:
      return 'mute-cuica'
    case 79:
      return 'open-cuica'
    case 80:
      return 'mute-triangle'
    case 81:
      return 'open-triangle'
    default:
      return undefined
  }
}
function manufacturerName(identifier: u32, extended: bool): Manufacturer | undefined {
  if (extended) return identifier === 0x2029 ? 'novation' : undefined
  switch (identifier) {
    case 1:
      return 'sequential'
    case 4:
      return 'moog'
    case 64:
      return 'kawai'
    case 65:
      return 'roland'
    case 66:
      return 'korg'
    case 67:
      return 'yamaha'
    case 68:
      return 'casio'
    case 71:
      return 'akai'
    case 125:
      return 'educational'
    case 126:
      return 'universal-non-realtime'
    case 127:
      return 'universal-realtime'
    case 8233:
      return 'novation'
    default:
      return undefined
  }
}
class MidiSemantics extends StreamDecoder<Config, MidiEvent, Event> {
  sysex: u64 | undefined = undefined
  count: u32 = 0
  identifier: u32 = 0
  extended = false
  override onGap(_range: Range): void {
    this.sysex = undefined
    this.count = 0
  }
  override async run(): Promise<void> {
    for (;;) {
      const received = await this.input.next()
      const e = received.event
      const at = received.at
      if (e.kind === 'channel-message') {
        if (e.status === 'note-on' || e.status === 'note-off' || e.status === 'poly-pressure') {
          let action: Action = 'on'
          if (e.status === 'note-off' || (e.status === 'note-on' && e.data2 === 0)) action = 'off'
          if (e.status === 'poly-pressure') action = 'pressure'
          const oct: u8 = e.data1 / 12
          await this.out.emit(
            {
              kind: 'note',
              message: e.message,
              channel: e.channel,
              note: e.data1,
              pitchClass: pitchClass(e.data1),
              octave: oct === 0 ? 1 : oct - 1,
              belowZero: oct === 0,
              action,
              value: e.data2,
              drum: e.channel === this.config.percussionChannel ? drumName(e.data1) : undefined,
            },
            at,
          )
        } else if (e.status === 'control-change') {
          const lsb = e.data1 >= 32 && e.data1 <= 63
          await this.out.emit(
            {
              kind: 'controller',
              message: e.message,
              channel: e.channel,
              number: e.data1,
              controller: controllerName(lsb ? e.data1 - 32 : e.data1),
              lsb,
              value: e.data2,
            },
            at,
          )
        } else if (e.status === 'pitch-bend') {
          const high = e.data2
          if (high !== undefined) {
            const value: u16 = (high as u16) * 128 + e.data1
            const negative = value < 8192
            await this.out.emit(
              {
                kind: 'pitch-bend',
                message: e.message,
                channel: e.channel,
                value,
                magnitude: negative ? 8192 - value : value - 8192,
                negative,
              },
              at,
            )
          }
        }
      } else if (e.kind === 'system-common') {
        const low = e.data1
        if (low !== undefined) {
          if (e.status === 'time-code-quarter-frame')
            await this.out.emit(
              { kind: 'quarter-frame', message: e.message, piece: low >> 4, value: low & 15 },
              at,
            )
          if (e.status === 'song-position-pointer') {
            const high = e.data2
            if (high !== undefined) {
              const value: u16 = (high as u16) * 128 + low
              await this.out.emit(
                {
                  kind: 'song-position',
                  message: e.message,
                  midiBeats: value,
                  midiClocks: (value as u32) * 6,
                },
                at,
              )
            }
          }
        }
      } else if (e.kind === 'sysex') {
        if (e.offset === 0) {
          this.sysex = e.message
          this.count = 0
          this.identifier = 0
          this.extended = false
        }
        if (this.sysex !== e.message || e.offset !== this.count) continue
        for (const byte of e.bytes) {
          if (this.count === 0) {
            this.extended = byte === 0
            this.identifier = byte
          } else if (this.extended && this.count < 3)
            this.identifier = (this.identifier << 8) | byte
          this.count += 1
          if ((!this.extended && this.count === 1) || (this.extended && this.count === 3))
            await this.out.emit(
              {
                kind: 'manufacturer',
                message: e.message,
                identifier: this.identifier,
                extended: this.extended,
                manufacturer: manufacturerName(this.identifier, this.extended),
              },
              at,
            )
        }
      } else if (e.kind === 'sysex-end') this.sysex = undefined
    }
  }
}
export const midiSemantics = defineStreamDecoder({
  name: 'midi-semantics',
  category: 'device',
  config: {
    defaults: { percussionChannel: 9 },
    validate: (config) => (config.percussionChannel > 15 ? 'decode.config.midiChannel' : undefined),
  },
  input: midiEvents,
  output: events,
  locales: {
    'en-US': {
      name: 'MIDI semantic fields',
      summary:
        'Note, drum and controller names, pitch bend, song position, quarter-frame and SysEx IDs',
      fields: { percussionChannel: 'Percussion channel (zero based)' },
      events: {
        note: 'Note',
        controller: 'Controller',
        'pitch-bend': 'Pitch bend',
        'song-position': 'Song position',
        'quarter-frame': 'Time code quarter frame',
        manufacturer: 'Manufacturer ID',
      },
      eventFields: {
        message: 'Message',
        channel: 'Channel',
        note: 'Note number',
        pitchClass: 'Pitch class',
        octave: 'Octave magnitude',
        belowZero: 'Negative octave',
        action: 'Action',
        value: 'Value',
        drum: 'GM percussion',
        number: 'Controller number',
        controller: 'Controller name',
        lsb: 'LSB controller',
        magnitude: 'Magnitude',
        negative: 'Negative',
        midiBeats: 'MIDI beats',
        midiClocks: 'MIDI clocks',
        piece: 'Quarter-frame piece',
        identifier: 'Identifier',
        extended: 'Extended identifier',
        manufacturer: 'Manufacturer',
      },
    },
    'zh-CN': {
      name: 'MIDI语义字段',
      summary: '音符、鼓组和控制器名称、弯音、乐曲位置、四分之一帧和SysEx标识',
      fields: { percussionChannel: '打击乐通道（从0开始）' },
      events: {
        note: '音符',
        controller: '控制器',
        'pitch-bend': '弯音',
        'song-position': '乐曲位置',
        'quarter-frame': '时间码四分之一帧',
        manufacturer: '制造商标识',
      },
      eventFields: {
        message: '消息',
        channel: '通道',
        note: '音符编号',
        pitchClass: '音级',
        octave: '八度绝对值',
        belowZero: '负八度',
        action: '动作',
        value: '值',
        drum: 'GM打击乐',
        number: '控制器编号',
        controller: '控制器名称',
        lsb: '低位控制器',
        magnitude: '绝对值',
        negative: '负方向',
        midiBeats: 'MIDI拍',
        midiClocks: 'MIDI时钟',
        piece: '四分之一帧片段',
        identifier: '标识',
        extended: '扩展标识',
        manufacturer: '制造商',
      },
    },
  },
  decoder: MidiSemantics,
})
