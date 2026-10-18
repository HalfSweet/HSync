<script setup lang="ts">
import { computed, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import TriggerPanel from '@/components/trigger/TriggerPanel.vue'
import { useFileActions } from '@/composables/useFileActions'
import { useShortcutLabel, useShortcuts } from '@/composables/useShortcuts'
import { useCaptureStore } from '@/stores/capture'
import { useSearchStore } from '@/stores/search'
import { formatTime } from '@/utils/units'

import AcquisitionMenu from './AcquisitionMenu.vue'
import CaptureButton from './CaptureButton.vue'
import DeviceMenu from './DeviceMenu.vue'
import ExportMenu from './ExportMenu.vue'
import SettingsButton from './SettingsButton.vue'

const { t } = useI18n()
const capture = useCaptureStore()
const finder = useSearchStore()
const search = useTemplateRef<HTMLInputElement>('search')

const shortcut = useShortcutLabel()

// Opening a capture file, saving the open capture and searching, by default ⌘/Ctrl+O, ⌘/Ctrl+S
// and `/` (DESIGN.md §8, §10).
const files = useFileActions()
useShortcuts({
  open: () => void files.open(),
  save: () => void files.save(),
  search: () => search.value?.focus(),
})

/** What the search found, at the right end of the box. */
const searchNote = computed<{ text: string; alert: boolean } | null>(() => {
  const note = (text: string, alert = false) => ({ text, alert })
  if (finder.kind === 'decode') {
    const result = finder.result
    if (!result) return null
    const total = result.hits.length
    if (total === 0) return note(t('header.search.none'))
    const count = result.truncated ? `${total}+` : String(total)
    return finder.current === null
      ? note(t('header.search.count', { count }))
      : note(t('header.search.position', { current: finder.current + 1, count }))
  }
  if (finder.kind === 'pattern') {
    const parsed = finder.pattern
    if (parsed?.kind === 'unknownChannel') {
      return note(t('header.search.unknownChannel', { name: parsed.name }), true)
    }
    if (parsed?.conditions.length === 0) return note(t('header.search.noCondition'))
    if (finder.exhausted) return note(t('header.search.noMore'))
    if (finder.found !== null) return note(formatTime(capture.timeOf(finder.found), 4))
    return note(t('header.search.pressEnter'))
  }
  return null
})

function onSearchKey(event: KeyboardEvent): void {
  if (event.key === 'Enter') {
    event.preventDefault()
    void finder.step(!event.shiftKey)
  } else if (event.key === 'Escape') {
    event.preventDefault()
    finder.clear()
    search.value?.blur()
  }
}
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

    <TriggerPanel />

    <CaptureButton />

    <div class="min-w-0 flex-1" />

    <label
      class="flex h-8 max-w-[340px] min-w-0 flex-[1_1_220px] items-center gap-2 rounded-6 bg-control pr-2 pl-3 text-ink-2 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ink"
    >
      <AppIcon name="search" />
      <input
        ref="search"
        v-model="finder.query"
        type="search"
        class="min-w-0 flex-1 border-0 bg-transparent p-0 text-body text-ink outline-none"
        :aria-label="t('header.search.aria')"
        :placeholder="t('header.search.placeholder')"
        :title="t('header.search.tip')"
        @keydown="onSearchKey"
      />
      <span
        v-if="searchNote"
        class="rise-in flex-none text-caption whitespace-nowrap tabular-nums"
        :class="searchNote.alert ? 'text-alert' : 'text-ink-3'"
        aria-live="polite"
        >{{ searchNote.text }}</span
      >
      <kbd
        v-else-if="shortcut('search')"
        class="min-w-5 flex-none rounded-4 bg-control px-1 text-center font-sans text-caption leading-[18px] text-ink-3"
      >
        {{ shortcut('search') }}
      </kbd>
    </label>

    <ExportMenu />
    <SettingsButton />
  </header>
</template>
