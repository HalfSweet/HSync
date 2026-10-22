// SPDX-License-Identifier: Apache-2.0
/** Exact MIDI 1.0 transport contract; interpretation belongs to consumers. */
import type { Bounded, bool, u8, u32, u64 } from 'uiharu'
import { eventSchema } from 'uiharu'
export type ChannelStatus =
  | 'note-off'
  | 'note-on'
  | 'poly-pressure'
  | 'control-change'
  | 'program-change'
  | 'channel-pressure'
  | 'pitch-bend'
export type CommonStatus =
  'time-code-quarter-frame' | 'song-position-pointer' | 'song-select' | 'tune-request'
export type RealtimeStatus =
  'clock' | 'start' | 'continue' | 'stop' | 'active-sensing' | 'reset' | 'undefined'
export type EndReason = 'end-of-exclusive' | 'interrupted' | 'aborted'
export type ErrorReason =
  'unexpected-data' | 'incomplete' | 'aborted' | 'undefined-status' | 'unexpected-end-of-exclusive'
export type MidiEvent =
  | {
      kind: 'channel-message'
      message: u64
      channel: u8
      status: ChannelStatus
      data1: u8
      data2: u8 | undefined
      running: bool
    }
  | {
      kind: 'system-common'
      message: u64
      status: CommonStatus
      data1: u8 | undefined
      data2: u8 | undefined
    }
  | { kind: 'realtime'; status: RealtimeStatus; byte: u8 }
  | { kind: 'sysex'; message: u64; offset: u32; bytes: Bounded<u8, 32> }
  | { kind: 'sysex-end'; message: u64; bytes: u32; reason: EndReason }
  | { kind: 'error'; reason: ErrorReason; byte: u8 }
export const midiEvents = eventSchema<MidiEvent>('midi.events', 1).roles({
  'channel-message': 'data',
  'system-common': 'control',
  realtime: 'control',
  sysex: 'data',
  'sysex-end': 'framing',
  error: 'error',
})
