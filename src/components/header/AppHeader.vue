<script setup lang="ts">
import { onKeyStroke } from '@vueuse/core'
import { computed, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import TriggerGlyph from '@/components/common/TriggerGlyph.vue'
import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { summarizeTriggers } from '@/timeline/triggerSummary'
import { formatSamplerate, formatTimeShort } from '@/utils/units'

import SettingsMenu from './SettingsMenu.vue'

const { t } = useI18n()
const capture = useCaptureStore()
const channels = useChannelsStore()
const search = useTemplateRef<HTMLInputElement>('search')

const deviceName = computed(() => {
  const source = capture.summary?.source
  if (!source) return t('header.device.select')
  if (source.kind === 'demo') return 'Demo Device'
  return source.path.split(/[\\/]/).pop() ?? source.path
})

const rate = computed(() => formatSamplerate(capture.samplerate))
const duration = computed(() => formatTimeShort(capture.duration))
const channelCount = computed(() => capture.summary?.channels.length ?? 0)
const acquisitionLabel = computed(() =>
  t('header.acquisition.aria', {
    rate: rate.value,
    duration: duration.value,
    channels: channelCount.value,
  }),
)

const trigger = computed(() => summarizeTriggers(channels.armed))
const triggerTip = computed(() => {
  if (channels.armed.length === 0) return t('header.trigger.tipNone')
  const conditions = channels.armed
    .map(({ index, condition }) =>
      t('header.trigger.tipItem', {
        channel: capture.channelLabel(index),
        condition: t(`trigger.condition.${condition}`),
      }),
    )
    .join(t('header.trigger.tipJoin'))
  return t('header.trigger.tipSet', { conditions })
})

// `/` focuses the search box from anywhere but text fields (DESIGN.md §9.8).
onKeyStroke('/', (event) => {
  const target = event.target as HTMLElement | null
  if (target?.closest('input, textarea, [contenteditable="true"]')) return
  event.preventDefault()
  search.value?.focus()
})
</script>

<template>
  <header class="flex flex-wrap items-center gap-2 border-b border-line px-3 py-2">
    <svg
      viewBox="0 0 20 20"
      aria-hidden="true"
      class="size-5 flex-none fill-none stroke-ink [stroke-width:1.75] [stroke-linecap:round] [stroke-linejoin:round]"
    >
      <path d="M3 13.5h3.5v-7h4v7h3.5v-7H17" />
    </svg>

    <button class="btn font-medium">
      <span>{{ deviceName }}</span>
      <AppIcon name="chevron-down" />
    </button>

    <button class="btn gap-3" :aria-label="acquisitionLabel">
      <span class="tabular-nums">{{ rate }}</span>
      <i class="vr" />
      <span class="tabular-nums">{{ duration }}</span>
      <i class="vr" />
      <span>{{ t('header.acquisition.channels', { count: channelCount }) }}</span>
      <AppIcon name="chevron-down" />
    </button>

    <button class="btn" :aria-label="triggerTip" :title="triggerTip">
      <AppIcon name="trigger" />
      <span>{{ t('header.trigger.label') }}</span>
      <span v-if="trigger.shown.length === 0" class="text-ink-3">{{
        t('header.trigger.none')
      }}</span>
      <template v-for="(item, position) in trigger.shown" :key="item.index">
        <span v-if="position > 0" class="text-ink-3">+</span>
        <span class="inline-flex items-center gap-0.5 font-medium">
          <span>{{ capture.channelLabel(item.index) }}</span>
          <TriggerGlyph :condition="item.condition" />
        </span>
      </template>
      <span v-if="trigger.more" class="text-ink-3">
        {{ t('header.trigger.more', { count: trigger.more }) }}
      </span>
      <AppIcon name="chevron-down" />
    </button>

    <div class="flex">
      <button
        class="inline-flex h-8 items-center gap-2 rounded-l-6 bg-ink pr-4 pl-3 font-medium whitespace-nowrap text-canvas hover:opacity-86"
        @click="capture.openDemo()"
      >
        <AppIcon name="play" />
        <span>{{ t('header.capture.start') }}</span>
      </button>
      <button
        class="ml-px inline-flex h-8 w-7 items-center justify-center rounded-r-6 bg-ink text-canvas hover:opacity-86"
        :aria-label="t('header.capture.more')"
      >
        <AppIcon name="chevron-down" />
      </button>
    </div>

    <div class="min-w-0 flex-1" />

    <label
      class="flex h-8 max-w-[340px] min-w-0 flex-[1_1_220px] items-center gap-2 rounded-6 border border-line-2 bg-raise pr-2 pl-3 text-ink-2 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ink"
    >
      <AppIcon name="search" />
      <input
        ref="search"
        type="search"
        class="min-w-0 flex-1 border-0 bg-transparent p-0 text-body text-ink outline-none"
        :aria-label="t('header.search.aria')"
        :placeholder="t('header.search.placeholder')"
      />
      <kbd
        class="min-w-5 flex-none rounded-4 border border-line-2 px-1 text-center font-sans text-caption leading-[18px] text-ink-3"
      >
        /
      </kbd>
    </label>

    <button
      class="btn w-8 justify-center p-0"
      :aria-label="t('header.export')"
      :title="t('header.export')"
    >
      <AppIcon name="export" />
    </button>
    <SettingsMenu />
  </header>
</template>
