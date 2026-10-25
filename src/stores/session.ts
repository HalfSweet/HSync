import { defineStore } from 'pinia'
import { nextTick, watch } from 'vue'

import { getBackend } from '@/ipc/backend'
import type { CapturePreset } from '@/ipc/bindings/CapturePreset'
import type { CaptureSummary } from '@/ipc/bindings/CaptureSummary'
import { toUiError, type UiError } from '@/ipc/errors'
import {
  optionRank,
  parseSession,
  readSession,
  SESSION_FORMAT,
  SESSION_VERSION,
  type Session,
} from '@/session/session'
import { newSerial, newStage, MAX_STAGES } from '@/trigger/advanced'

import { useCaptureStore } from './capture'
import { useChannelsStore } from './channels'
import { useDecodeStore } from './decode'
import { useDeviceStore } from './device'
import { useTriggerStore } from './trigger'
import { useViewStore } from './view'

/** Where the last settings of each driver are kept between runs. */
const STORAGE_PREFIX = 'hsync.session.'

function remember(driver: string, session: Session): void {
  try {
    localStorage.setItem(STORAGE_PREFIX + driver, JSON.stringify(session))
  } catch {
    // Without storage the settings are simply not remembered.
  }
}

function recall(driver: string): Session | null {
  try {
    const stored = localStorage.getItem(STORAGE_PREFIX + driver)
    return stored ? parseSession(stored) : null
  } catch {
    return null
  }
}

/** Channels of a capture with names of their own. */
function namesOf(summary: CaptureSummary): Record<string, string> {
  return Object.fromEntries(
    summary.channels
      .filter((channel) => channel.name !== String(channel.index))
      .map((channel) => [channel.index, channel.name]),
  )
}

/** The channel names and decoders a session gives a first capture; `null` for none. */
function presetOf(session: Session): CapturePreset | null {
  const { channelNames, decoders } = session
  return decoders === null && Object.keys(channelNames).length === 0
    ? null
    : { channelNames: { ...channelNames }, decoders }
}

/**
 * Sessions: the device settings, channels, triggers and decoders as a whole,
 * saved to and loaded from files, and remembered per driver between runs, as DSView restores a
 * device's last settings. Channel names and decoders belong to captures: those remembered, or of
 * a session loaded before any capture, wait for the first one.
 */
export const useSessionStore = defineStore('session', () => {
  const device = useDeviceStore()
  const channels = useChannelsStore()
  const trigger = useTriggerStore()
  const capture = useCaptureStore()
  const decode = useDecodeStore()
  const view = useViewStore()

  /** Everything a session holds, as things are now. */
  function snapshot(): Session {
    const state = device.state
    const summary = capture.summary
    const decoders = summary ? decode.entries : (capture.preset?.decoders ?? [])
    return {
      format: SESSION_FORMAT,
      version: SESSION_VERSION,
      device: state ? { driver: state.info.driver, model: state.info.model } : null,
      options: (state?.options ?? [])
        .filter((option) => option.enabled)
        .sort((a, b) => optionRank(a.key) - optionRank(b.key))
        .map((option) => [option.key, option.value]),
      enabledChannels: state ? [...state.enabledChannels] : null,
      analogInputs: state
        ? state.analogInputs.filter((input) => input.enabled).map((input) => input.index)
        : null,
      channelNames: summary ? namesOf(summary) : { ...capture.preset?.channelNames },
      colours: { ...channels.colours } as Record<string, number>,
      heights: { ...channels.heights } as Record<string, number>,
      triggers: { ...channels.triggers },
      trigger: {
        mode: trigger.mode,
        position: trigger.position,
        stageCount: trigger.stageCount,
        stages: trigger.stages,
        serial: trigger.serial,
      },
      decoders: decoders.length > 0 ? [...decoders] : null,
      captureMode: capture.mode,
      view: null,
    }
  }

  /** The session to store with the open capture: the settings, and the timeline as shown. */
  function captureDocument(): Session {
    const marked = view.segment
    return {
      ...snapshot(),
      view: {
        start: view.viewport.start,
        end: view.end,
        segment: marked ? { from: marked.from, to: marked.to } : null,
      },
    }
  }

  /**
   * Applies what an `.hsync` capture that has just opened stored of the interface: the channel
   * colours and heights, and the timeline as it was. Its channel names and decoders came with
   * the capture; the device settings it holds stay unapplied, as opening a file must not change
   * the device.
   */
  async function applyCaptureDocument(document: unknown): Promise<void> {
    const session = readSession(document)
    if (!session) return
    channels.colours = { ...session.colours }
    channels.heights = { ...session.heights }
    const stored = session.view
    if (!stored) return
    // The view store fits a capture once it opens; the stored view comes after that.
    await nextTick()
    view.showRange(stored.start, stored.end)
    view.setSegment(stored.segment)
  }

  /** Applies the device settings, channels and triggers of `session`. */
  async function applySettings(session: Session): Promise<void> {
    const state = device.state
    if (state && (!session.device || session.device.driver === state.info.driver)) {
      const options = [...session.options].sort((a, b) => optionRank(a[0]) - optionRank(b[0]))
      // Some devices allow a rate only with fewer channels on (ATK-Logic): what they refuse
      // now is tried again once the channels are set.
      const refused: typeof options = []
      for (const [key, value] of options) {
        if (device.option(key) && !(await device.setOption(key, value))) refused.push([key, value])
      }
      if (session.enabledChannels) {
        const wanted = new Set(session.enabledChannels)
        for (let index = 0; index < (device.state?.channelCount ?? 0); index++) {
          const enabled = device.enabledChannels.includes(index)
          if (enabled !== wanted.has(index)) await device.setChannel(index, wanted.has(index))
        }
      }
      if (session.analogInputs) {
        const wanted = new Set(session.analogInputs)
        for (const input of device.analogInputs) {
          if (input.enabled !== wanted.has(input.index))
            await device.setAnalog(input.index, wanted.has(input.index))
        }
      }
      for (const [key, value] of refused) await device.setOption(key, value)
    }
    channels.triggers = { ...session.triggers }
    channels.colours = { ...session.colours }
    channels.heights = { ...session.heights }
    const settings = session.trigger
    if (settings) {
      if (settings.mode) trigger.mode = settings.mode
      if (typeof settings.position === 'number') trigger.position = settings.position
      if (typeof settings.stageCount === 'number') trigger.stageCount = settings.stageCount
      if (Array.isArray(settings.stages)) {
        trigger.stages = Array.from({ length: MAX_STAGES }, (_, index) => ({
          ...newStage(),
          ...settings.stages[index],
        }))
      }
      if (settings.serial) trigger.serial = { ...newSerial(), ...settings.serial }
    }
    // After the options, which decide whether the device can loop.
    const mode = session.captureMode
    if (mode && (mode !== 'loop' || capture.canLoop)) capture.mode = mode
  }

  /**
   * Applies all of `session`, its channel names and decoders to the open capture too, or
   * without one to the first capture.
   */
  async function apply(session: Session): Promise<void> {
    await applySettings(session)
    const summary = capture.summary
    if (!summary) {
      // As with a capture, a session without decoders leaves those there are.
      capture.preset = presetOf({
        ...session,
        decoders: session.decoders ?? capture.preset?.decoders ?? null,
      })
      return
    }
    for (const channel of summary.channels) {
      const name = session.channelNames[channel.index] ?? ''
      const current = channel.name === String(channel.index) ? '' : channel.name
      if (name !== current) await capture.renameChannel(channel.index, name)
    }
    if (session.decoders) await decode.setEntries(session.decoders)
  }

  /** Writes the session to `path`. */
  function save(path: string): Promise<boolean> {
    return capture.saveText(path, `${JSON.stringify(snapshot(), null, 2)}\n`)
  }

  /** Reads a session from `path` and applies it; the reason when it could not. */
  async function load(path: string): Promise<UiError | null> {
    try {
      const session = parseSession(await (await getBackend()).readText(path))
      if (!session) return { code: 'error.file.notSession', params: {} }
      await apply(session)
      return null
    } catch (reason) {
      return toUiError(reason)
    }
  }

  // Each driver's settings are remembered as they change, and come back when it opens again.
  let restoring = false
  watch(
    () => device.state?.info.id,
    async (id) => {
      const driver = device.state?.info.driver
      if (id === undefined || !driver) return
      const stored = recall(driver)
      capture.preset = stored && presetOf(stored)
      if (!stored) return
      restoring = true
      try {
        await applySettings(stored)
      } finally {
        restoring = false
      }
    },
  )
  let timer: ReturnType<typeof setTimeout> | undefined
  watch(
    () =>
      [
        device.state,
        channels.triggers,
        channels.colours,
        channels.heights,
        trigger.$state,
        capture.summary?.channels,
        capture.summary?.decoders,
        capture.preset,
      ] as const,
    () => {
      const driver = device.state?.info.driver
      if (restoring || !driver) return
      clearTimeout(timer)
      timer = setTimeout(() => remember(driver, remembered(driver)), 500)
    },
    { deep: true },
  )

  /**
   * What to remember of `driver`: the settings, and the channel names and decoders of its own
   * capture, kept while a file or another device's capture is shown. An empty decoder list
   * remembers that there were none.
   */
  function remembered(driver: string): Session {
    const settings = snapshot()
    const summary = capture.summary
    if (!summary) {
      const preset = capture.preset
      return {
        ...settings,
        channelNames: { ...preset?.channelNames },
        decoders: preset?.decoders ?? null,
      }
    }
    const source = summary.source
    if (source.kind === 'device' && source.driver === driver) {
      return { ...settings, channelNames: namesOf(summary), decoders: [...decode.entries] }
    }
    const kept = recall(driver)
    return { ...settings, channelNames: kept?.channelNames ?? {}, decoders: kept?.decoders ?? null }
  }

  return { snapshot, apply, save, load, captureDocument, applyCaptureDocument }
})
