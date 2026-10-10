<script setup lang="ts">
import { onKeyStroke } from '@vueuse/core'
import { computed, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import TriggerGlyph from '@/components/common/TriggerGlyph.vue'
import { useFileActions } from '@/composables/useFileActions'
import { useCaptureStore } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { summarizeTriggers } from '@/timeline/triggerSummary'
import { isCommand } from '@/utils/keys'

import AcquisitionMenu from './AcquisitionMenu.vue'
import CaptureButton from './CaptureButton.vue'
import DeviceMenu from './DeviceMenu.vue'
import ExportMenu from './ExportMenu.vue'
import SettingsMenu from './SettingsMenu.vue'

const { t } = useI18n()
const capture = useCaptureStore()
const channels = useChannelsStore()
const search = useTemplateRef<HTMLInputElement>('search')

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

// ⌘/Ctrl+O opens a capture file and ⌘/Ctrl+S saves the open capture.
const files = useFileActions()
onKeyStroke(
  (event) => isCommand(event, 'o') || isCommand(event, 's'),
  (event) => {
    event.preventDefault()
    void (isCommand(event, 'o') ? files.open() : files.save())
  },
)

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

    <DeviceMenu />

    <AcquisitionMenu />

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

    <CaptureButton />

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

    <ExportMenu />
    <SettingsMenu />
  </header>
</template>
