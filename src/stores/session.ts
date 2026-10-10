import { defineStore } from 'pinia'
import { watch } from 'vue'

import { getBackend } from '@/ipc/backend'
import { toUiError, type UiError } from '@/ipc/errors'
import {
  optionRank,
  parseSession,
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

/**
 * Sessions (DESIGN.md §8.3): the device settings, channels, triggers and decoders as a whole,
 * saved to and loaded from files, and the device settings remembered per driver between runs,
 * as DSView restores a device's last settings.
 */
export const useSessionStore = defineStore('session', () => {
  const device = useDeviceStore()
  const channels = useChannelsStore()
  const trigger = useTriggerStore()
  const capture = useCaptureStore()
  const decode = useDecodeStore()

  /** Everything a session holds, as things are now. */
  function snapshot(): Session {
    const state = device.state
    return {
      format: SESSION_FORMAT,
      version: SESSION_VERSION,
      device: state ? { driver: state.info.driver, model: state.info.model } : null,
      options: (state?.options ?? [])
        .filter((option) => option.enabled)
        .sort((a, b) => optionRank(a.key) - optionRank(b.key))
        .map((option) => [option.key, option.value]),
      enabledChannels: state ? [...state.enabledChannels] : null,
      channelNames: Object.fromEntries(
        (capture.summary?.channels ?? [])
          .filter((channel) => channel.name !== String(channel.index))
          .map((channel) => [channel.index, channel.name]),
      ),
      colours: { ...channels.colours } as Record<string, number>,
      triggers: { ...channels.triggers },
      trigger: {
        mode: trigger.mode,
        position: trigger.position,
        stageCount: trigger.stageCount,
        stages: trigger.stages,
        serial: trigger.serial,
      },
      decoders: decode.entries.length > 0 ? [...decode.entries] : null,
      captureMode: capture.mode,
    }
  }

  /** Applies the device settings, channels and triggers of `session`. */
  async function applySettings(session: Session): Promise<void> {
    const state = device.state
    if (state && (!session.device || session.device.driver === state.info.driver)) {
      const options = [...session.options].sort((a, b) => optionRank(a[0]) - optionRank(b[0]))
      for (const [key, value] of options) {
        if (device.option(key)) await device.setOption(key, value)
      }
      if (session.enabledChannels) {
        const wanted = new Set(session.enabledChannels)
        for (let index = 0; index < (device.state?.channelCount ?? 0); index++) {
          const enabled = device.enabledChannels.includes(index)
          if (enabled !== wanted.has(index)) await device.setChannel(index, wanted.has(index))
        }
      }
    }
    channels.triggers = { ...session.triggers }
    channels.colours = { ...session.colours }
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

  /** Applies all of `session`, its channel names and decoders to the open capture too. */
  async function apply(session: Session): Promise<void> {
    await applySettings(session)
    const summary = capture.summary
    if (!summary) return
    for (const channel of summary.channels) {
      const name = session.channelNames[channel.index] ?? ''
      const current = channel.name === String(channel.index) ? '' : channel.name
      if (name !== current) await capture.renameChannel(channel.index, name)
    }
    if (session.decoders) await decode.setEntries(session.decoders)
  }

  /** Writes the session to `path`. */
  async function save(path: string): Promise<boolean> {
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
    () => [device.state, channels.triggers, channels.colours, trigger.$state] as const,
    () => {
      const driver = device.state?.info.driver
      if (restoring || !driver) return
      clearTimeout(timer)
      timer = setTimeout(() => {
        const { decoders: _, channelNames: __, ...settings } = snapshot()
        remember(driver, { ...settings, decoders: null, channelNames: {} })
      }, 500)
    },
    { deep: true },
  )

  return { snapshot, apply, save, load }
})
