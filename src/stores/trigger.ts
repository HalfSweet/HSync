import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import type { CaptureRequest } from '@/ipc/bindings/CaptureRequest'
import type { TriggerCaps } from '@/ipc/bindings/TriggerCaps'
import {
  MAX_STAGES,
  advancedTrigger,
  newSerial,
  newStage,
  type SerialDraft,
  type StageDraft,
  type TriggerMode,
} from '@/trigger/advanced'

import { useChannelsStore } from './channels'
import { useDeviceStore } from './device'

/** What the trigger decides of a capture request; the capture store adds how to capture. */
export type TriggerRequest = Omit<CaptureRequest, 'looping'>

/** What a trigger can do without a device to ask: simple triggers up to 90 %, as DSView. */
const DEFAULT_CAPS: TriggerCaps = { maxPosition: 90, advanced: false }

/**
 * The trigger panel's settings (DESIGN.md §2.2, §15 item 5): the trigger position, and a
 * multi-stage or serial trigger in place of the per-channel conditions of the timeline.
 */
export const useTriggerStore = defineStore('trigger', () => {
  const device = useDeviceStore()
  const channels = useChannelsStore()

  const mode = ref<TriggerMode>('simple')
  /** Share of a triggered capture before the trigger, in percent; DSView starts at 1. */
  const position = ref(1)
  const stageCount = ref(1)
  const stages = ref<StageDraft[]>(Array.from({ length: MAX_STAGES }, newStage))
  const serial = ref<SerialDraft>(newSerial())

  const caps = computed(() => device.state?.trigger ?? DEFAULT_CAPS)
  const channelCount = computed(() => device.state?.channelCount ?? 16)
  /** The trigger a capture uses: multi-stage and serial only where the device offers them. */
  const activeMode = computed<TriggerMode>(() => (caps.value.advanced ? mode.value : 'simple'))
  const activePosition = computed(() =>
    Math.min(Math.max(1, Math.round(position.value)), caps.value.maxPosition),
  )

  /** The trigger part of the next capture's request. */
  function request(instant: boolean): TriggerRequest {
    return {
      triggers: channels.armed.map(({ index, condition }) => [index, condition]),
      triggerPosition: activePosition.value,
      instant,
      advanced: advancedTrigger(
        activeMode.value,
        stages.value,
        stageCount.value,
        serial.value,
        channelCount.value,
      ),
    }
  }

  return {
    mode,
    position,
    stageCount,
    stages,
    serial,
    caps,
    channelCount,
    activeMode,
    activePosition,
    request,
  }
})
