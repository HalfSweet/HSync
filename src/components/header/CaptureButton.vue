<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import type { CaptureRequest } from '@/ipc/bindings/CaptureRequest'
import { useCaptureStore, type CaptureMode } from '@/stores/capture'
import { useChannelsStore } from '@/stores/channels'
import { useDeviceStore } from '@/stores/device'

/** Capture / stop, and how to capture (DESIGN.md §9.4). */
const { t } = useI18n()
const capture = useCaptureStore()
const channels = useChannelsStore()
const device = useDeviceStore()
const open = ref(false)
const root = useTemplateRef<HTMLElement>('root')

onClickOutside(root, () => (open.value = false))
onKeyStroke('Escape', () => (open.value = false))

const MODES: CaptureMode[] = ['single', 'repeat']
const ready = computed(() => device.state !== null && device.opening === null)

function request(instant: boolean): CaptureRequest {
  return {
    triggers: channels.armed.map(({ index, condition }) => [index, condition]),
    triggerPosition: channels.triggerPosition,
    instant,
  }
}

function run(): void {
  if (capture.busy) void capture.stop()
  else void capture.start(request(false))
}

function setMode(mode: CaptureMode): void {
  capture.mode = mode
  open.value = false
}

function instant(): void {
  open.value = false
  if (!capture.busy) void capture.start(request(true))
}
</script>

<template>
  <div ref="root" class="relative flex">
    <button
      class="inline-flex h-8 items-center gap-2 rounded-l-6 bg-ink pr-4 pl-3 font-medium whitespace-nowrap text-canvas hover:opacity-86 disabled:opacity-55"
      :disabled="!ready"
      @click="run"
    >
      <AppIcon :name="capture.busy ? 'stop' : 'play'" />
      <span>{{ capture.busy ? t('header.capture.stop') : t('header.capture.start') }}</span>
    </button>
    <button
      class="ml-px inline-flex h-8 w-7 items-center justify-center rounded-r-6 bg-ink text-canvas hover:opacity-86"
      :aria-label="t('header.capture.more')"
      aria-haspopup="menu"
      :aria-expanded="open"
      @click="open = !open"
    >
      <AppIcon name="chevron-down" />
    </button>
    <div
      v-if="open"
      role="menu"
      :aria-label="t('header.capture.more')"
      class="absolute top-full right-0 z-20 mt-1 flex w-64 flex-col rounded-6 border border-ink bg-raise p-1 text-small"
    >
      <button
        v-for="mode in MODES"
        :key="mode"
        role="menuitemradio"
        :aria-checked="capture.mode === mode"
        class="flex min-h-[30px] w-full items-center gap-2 rounded-4 px-2 py-1 text-left hover:bg-hover"
        @click="setMode(mode)"
      >
        <span class="flex min-w-0 flex-1 flex-col">
          <span :class="capture.mode === mode && 'font-medium'">{{
            t(`header.capture.${mode}`)
          }}</span>
          <span class="text-caption text-ink-3">{{ t(`header.capture.${mode}Tip`) }}</span>
        </span>
        <AppIcon v-if="capture.mode === mode" name="check" />
      </button>
      <button
        role="menuitemradio"
        aria-checked="false"
        aria-disabled="true"
        class="flex min-h-[30px] w-full cursor-default items-center gap-2 rounded-4 px-2 py-1 text-left opacity-55"
      >
        <span class="flex min-w-0 flex-1 flex-col">
          <span>{{ t('header.capture.loop') }}</span>
          <span class="text-caption text-ink-3">{{ t('header.capture.notYet') }}</span>
        </span>
      </button>
      <div class="my-1 border-t border-line" />
      <button
        role="menuitem"
        class="flex min-h-[30px] w-full items-center gap-2 rounded-4 px-2 py-1 text-left hover:bg-hover disabled:opacity-55 disabled:hover:bg-transparent"
        :disabled="!ready || capture.busy"
        @click="instant"
      >
        <span class="flex min-w-0 flex-1 flex-col">
          <span>{{ t('header.capture.instant') }}</span>
          <span class="text-caption text-ink-3">{{ t('header.capture.instantTip') }}</span>
        </span>
      </button>
    </div>
  </div>
</template>
