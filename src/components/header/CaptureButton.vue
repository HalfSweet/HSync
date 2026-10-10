<script setup lang="ts">
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import { computed, ref, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { REPEAT_INTERVAL, useCaptureStore, type CaptureMode } from '@/stores/capture'
import { useDeviceStore } from '@/stores/device'
import { useTriggerStore } from '@/stores/trigger'

/** Capture / stop, and how to capture (DESIGN.md §4). */
const { t } = useI18n()
const capture = useCaptureStore()
const trigger = useTriggerStore()
const device = useDeviceStore()
const open = ref(false)
const root = useTemplateRef<HTMLElement>('root')

onClickOutside(root, () => (open.value = false))
onKeyStroke('Escape', () => (open.value = false))

const MODES: CaptureMode[] = ['single', 'repeat', 'loop']
const ready = computed(() => device.state !== null && device.opening === null)

function run(): void {
  if (capture.busy) void capture.stop()
  else void capture.start(trigger.request(false))
}

/** Loop captures need a device that can run on, such as a DSLogic in Stream mode. */
const unavailable = (mode: CaptureMode) => mode === 'loop' && !capture.canLoop

function setMode(mode: CaptureMode): void {
  if (unavailable(mode)) return
  capture.mode = mode
  open.value = false
}

function instant(): void {
  open.value = false
  if (!capture.busy) void capture.start(trigger.request(true))
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
    <Transition name="pop">
      <div
        v-if="open"
        role="menu"
        :aria-label="t('header.capture.more')"
        class="absolute top-full right-0 z-20 mt-1 flex w-64 origin-top-right flex-col rounded-6 border border-ink bg-raise p-1 text-small"
      >
        <button
          v-for="mode in MODES"
          :key="mode"
          role="menuitemradio"
          :aria-checked="capture.mode === mode"
          :aria-disabled="unavailable(mode) || undefined"
          class="flex min-h-[30px] w-full items-center gap-2 rounded-4 px-2 py-1 text-left"
          :class="unavailable(mode) ? 'cursor-default opacity-55' : 'hover:bg-hover'"
          @click="setMode(mode)"
        >
          <span class="flex min-w-0 flex-1 flex-col">
            <span :class="capture.mode === mode && 'font-medium'">{{
              t(`header.capture.${mode}`)
            }}</span>
            <span class="text-caption text-ink-3">{{
              unavailable(mode)
                ? t('header.capture.loopUnavailable')
                : t(`header.capture.${mode}Tip`)
            }}</span>
          </span>
          <AppIcon v-if="capture.mode === mode" name="check" />
        </button>
        <label
          v-if="capture.mode === 'repeat'"
          class="flex min-h-[30px] items-center gap-2 rounded-4 px-2 py-1 text-caption text-ink-3"
        >
          <span class="flex-1">{{ t('header.capture.interval') }}</span>
          <input
            v-model.lazy.number="capture.interval"
            type="number"
            class="h-6 w-16 rounded-4 border border-line-2 bg-raise px-1.5 text-right text-small text-ink tabular-nums"
            :min="REPEAT_INTERVAL.min"
            :max="REPEAT_INTERVAL.max"
            step="0.1"
            :aria-label="t('header.capture.interval')"
          />
          <span>{{ t('header.capture.seconds') }}</span>
        </label>
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
    </Transition>
  </div>
</template>
