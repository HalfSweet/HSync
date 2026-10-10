<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import SegmentMeasure from '@/components/status/SegmentMeasure.vue'
import { useBackendText } from '@/composables/useBackendText'
import { useCaptureStore } from '@/stores/capture'
import { useDeviceStore } from '@/stores/device'
import { useStreamStore } from '@/stores/stream'
import { useViewStore } from '@/stores/view'
import { formatCount, formatTimeShort } from '@/utils/units'

const { t, locale } = useI18n()
const text = useBackendText()
const capture = useCaptureStore()
const device = useDeviceStore()
const stream = useStreamStore()
const view = useViewStore()

const percent = (part: number, whole: number) =>
  whole > 0 ? Math.min(100, Math.floor((part / whole) * 100)) : 0

/** Where the capture is (DESIGN.md §9.10): waiting, how much arrived, or how it ended. */
const status = computed(() => {
  const live = capture.live
  if (live && live.captureId === capture.summary?.id) {
    switch (live.phase) {
      case 'waiting':
        return live.hardwareSamples === null
          ? t('status.waitingTrigger')
          : `${t('status.waitingTrigger')} · ${t('status.hardwareFill', {
              percent: percent(live.hardwareSamples, live.limit),
            })}`
      case 'capturing':
        return t('status.captured', { percent: percent(live.samples, live.limit) })
      default:
        if (live.end === 'stopped') return t('status.stopped')
        if (live.end === 'completed') return t('status.complete')
        return t('status.failed')
    }
  }
  switch (capture.status) {
    case 'loading':
      return t('status.capturing')
    case 'ready':
      return t('status.complete')
    default:
      return t('status.ready')
  }
})

/** The latest problem worth telling: a failed capture, lost data or a refused setting. */
const problem = computed(() => {
  if (capture.error) return text.error(capture.error)
  if (capture.live?.overflow && capture.live.captureId === capture.summary?.id)
    return t('status.overflow')
  if (device.error) return text.error(device.error)
  return null
})

const savedName = computed(() => capture.savedPath?.split(/[\\/]/).pop() ?? null)

const triggerTime = computed(() => {
  const ms = capture.summary?.info.triggerTimeMs
  if (ms == null) return null
  const date = new Date(ms)
  const pad = (value: number) => String(value).padStart(2, '0')
  const text = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`
  return t('status.triggerTime', { time: text })
})

const perDivision = computed(() =>
  view.ruler.step > 0 ? t('status.perDivision', { value: formatTimeShort(view.ruler.step) }) : '',
)
</script>

<template>
  <footer
    class="flex min-h-(--h-status) flex-wrap items-center gap-x-4 gap-y-1 border-t border-line px-3 py-0.5 text-small text-ink-2"
  >
    <button
      class="-ml-1 inline-flex h-[22px] items-center gap-1 rounded-4 pr-2 pl-1 whitespace-nowrap text-ink-2 hover:bg-hover hover:text-ink aria-pressed:bg-hover aria-pressed:text-ink"
      :aria-pressed="stream.open"
      @click="stream.toggle()"
    >
      <AppIcon name="stream" />
      <span>{{ t('stream.title') }}</span>
    </button>
    <span :aria-live="capture.capturing ? 'off' : 'polite'">{{ status }}</span>
    <span v-if="problem" role="alert" class="text-alert">{{ problem }}</span>
    <span v-else-if="savedName" role="status" class="text-ink-3">{{
      t('file.saved', { name: savedName })
    }}</span>
    <span v-if="triggerTime" class="text-ink-3 tabular-nums">{{ triggerTime }}</span>
    <span v-if="capture.summary" class="text-ink-3 tabular-nums">
      {{ t('status.samples', { count: formatCount(capture.sampleCount, locale) }) }}
    </span>
    <SegmentMeasure />
    <div class="flex-1" />
    <span v-if="device.arrival" role="status" class="inline-flex items-center gap-1.5">
      <span class="text-ink">{{
        t('header.device.arrived', { device: device.arrival.model })
      }}</span>
      <button class="btn-ghost h-[22px]" @click="device.acceptArrival()">
        {{ t('header.device.switch') }}
      </button>
      <button class="btn-ghost h-[22px]" @click="device.dismissArrival()">
        {{ t('header.device.dismiss') }}
      </button>
    </span>
    <div class="flex items-center gap-0.5">
      <button
        class="btn-icon"
        :aria-label="t('status.zoomOut')"
        :title="t('status.zoomOut')"
        @click="view.zoom(0.5)"
      >
        <AppIcon name="minus" />
      </button>
      <span class="min-w-16 text-center tabular-nums">{{ perDivision }}</span>
      <button
        class="btn-icon"
        :aria-label="t('status.zoomIn')"
        :title="t('status.zoomIn')"
        @click="view.zoom(2)"
      >
        <AppIcon name="plus" />
      </button>
      <button
        class="ml-1 btn-icon"
        :aria-label="t('status.zoomFit')"
        :title="t('status.zoomFit')"
        @click="view.fit()"
      >
        <AppIcon name="fit" />
      </button>
    </div>
  </footer>
</template>
