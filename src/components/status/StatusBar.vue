<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { useCaptureStore } from '@/stores/capture'
import { useViewStore } from '@/stores/view'
import { formatCount, formatTimeShort } from '@/utils/units'

const { t, locale } = useI18n()
const capture = useCaptureStore()
const view = useViewStore()

const status = computed(() => {
  switch (capture.status) {
    case 'loading':
      return t('status.capturing')
    case 'ready':
      return t('status.complete')
    default:
      return t('status.ready')
  }
})

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
      class="-ml-1 inline-flex h-[22px] items-center gap-1 rounded-4 pr-2 pl-1 whitespace-nowrap text-ink-2 hover:bg-hover hover:text-ink"
      aria-pressed="false"
    >
      <AppIcon name="stream" />
      <span>{{ t('stream.title') }}</span>
    </button>
    <span>{{ status }}</span>
    <span v-if="triggerTime" class="text-ink-3 tabular-nums">{{ triggerTime }}</span>
    <span v-if="capture.summary" class="text-ink-3 tabular-nums">
      {{ t('status.samples', { count: formatCount(capture.sampleCount, locale) }) }}
    </span>
    <div class="flex-1" />
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
