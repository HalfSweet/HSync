<script setup lang="ts">
import { onKeyStroke } from '@vueuse/core'
import { computed, onMounted, ref, shallowRef, useTemplateRef } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { EXTENSIONS, useFileActions } from '@/composables/useFileActions'
import type { ExportKind } from '@/ipc/bindings/ExportKind'
import { vIndicator } from '@/motion/indicator'
import { useCaptureStore } from '@/stores/capture'
import { useViewStore } from '@/stores/view'

/**
 * The export dialog (DESIGN.md §8.2): a format, the whole capture or the segment, and the
 * channels. Choosing where to write comes after, in the system's save dialog.
 */
const emit = defineEmits<{ close: [] }>()

const { t } = useI18n()
const capture = useCaptureStore()
const view = useViewStore()
const files = useFileActions()
const dialog = useTemplateRef<HTMLElement>('dialog')

onKeyStroke('Escape', () => emit('close'))
onMounted(() => dialog.value?.focus())

const KINDS: ExportKind[] = ['dsl', 'csv', 'vcd', 'sigrok', 'binary']
const kind = shallowRef<ExportKind>('csv')
const everySample = shallowRef(false)

const segment = computed<[number, number] | null>(() => {
  const marked = view.segment
  if (!marked || marked.from === marked.to) return null
  return [Math.min(marked.from, marked.to), Math.max(marked.from, marked.to)]
})
const onlySegment = shallowRef(segment.value !== null)

/** Channels in timeline order, all chosen at first. */
const channels = computed(() =>
  capture.groups.flatMap((group) =>
    group.channels.map((channel) => ({ index: channel.index, label: channel.label })),
  ),
)
const chosen = ref<number[]>(channels.value.map((channel) => channel.index))

const ready = computed(() => chosen.value.length > 0)

async function run(): Promise<void> {
  if (!ready.value) return
  const order = channels.value
    .map((channel) => channel.index)
    .filter((index) => chosen.value.includes(index))
  const range = onlySegment.value && segment.value ? segment.value : null
  emit('close')
  await files.exportTo({
    kind: kind.value,
    everySample: kind.value === 'csv' && everySample.value,
    range,
    channels: order,
  })
}
</script>

<template>
  <div
    class="fixed inset-0 z-40 flex items-center justify-center bg-veil"
    @click.self="emit('close')"
  >
    <div
      ref="dialog"
      role="dialog"
      aria-modal="true"
      :aria-label="t('exportDialog.title')"
      tabindex="-1"
      class="flex max-h-[85vh] w-[min(480px,calc(100vw-32px))] flex-col rounded-6 border border-ink bg-raise text-small outline-none"
    >
      <header
        class="flex h-10 flex-none items-center justify-between border-b border-line pr-2 pl-4"
      >
        <h2 class="font-medium">{{ t('exportDialog.title') }}</h2>
        <button class="btn-icon" :aria-label="t('exportDialog.cancel')" @click="emit('close')">
          <AppIcon name="close" />
        </button>
      </header>
      <div class="flex min-h-0 flex-col gap-4 overflow-y-auto p-4">
        <fieldset class="flex flex-col gap-1">
          <legend class="mb-1 text-caption text-ink-3">{{ t('exportDialog.format') }}</legend>
          <template v-for="one in KINDS" :key="one">
            <label
              class="flex cursor-pointer items-start gap-2 rounded-4 px-2 py-1.5 hover:bg-hover"
            >
              <input
                v-model="kind"
                type="radio"
                name="format"
                :value="one"
                class="mt-0.5 accent-ink"
              />
              <span class="flex flex-col">
                <span class="font-medium"
                  >{{ t(`exportDialog.formats.${one}`) }}
                  <span class="font-mono text-caption text-ink-3"
                    >.{{ EXTENSIONS[one] }}</span
                  ></span
                >
                <span class="text-ink-3">{{ t(`exportDialog.formatTips.${one}`) }}</span>
              </span>
            </label>
            <label
              v-if="one === 'csv' && kind === 'csv'"
              class="-mt-1 mb-1 flex items-center gap-2 pl-8 text-ink-2"
            >
              <input v-model="everySample" type="checkbox" class="accent-ink" />
              {{ t('exportDialog.everySample') }}
            </label>
          </template>
        </fieldset>

        <div class="flex items-center gap-3">
          <span class="w-12 flex-none text-caption text-ink-3">{{ t('exportDialog.range') }}</span>
          <div v-indicator role="group" class="flex gap-0.5 rounded-6 bg-hover p-0.5">
            <button class="seg" :aria-pressed="!onlySegment" @click="onlySegment = false">
              {{ t('exportDialog.rangeAll') }}
            </button>
            <button
              class="seg"
              :aria-pressed="onlySegment && segment !== null"
              :aria-disabled="segment === null"
              :title="segment === null ? t('exportDialog.segmentNeeded') : undefined"
              @click="segment !== null && (onlySegment = true)"
            >
              {{ t('exportDialog.rangeSegment') }}
            </button>
          </div>
        </div>

        <fieldset class="flex flex-col gap-1.5">
          <legend class="mb-1 flex w-full items-center gap-2 text-caption text-ink-3">
            <span class="flex-1">{{ t('exportDialog.channels') }}</span>
            <button
              class="hover:text-ink"
              @click="chosen = channels.map((channel) => channel.index)"
            >
              {{ t('exportDialog.all') }}
            </button>
            <button class="hover:text-ink" @click="chosen = []">
              {{ t('exportDialog.none') }}
            </button>
          </legend>
          <div class="grid grid-cols-4 gap-x-2 gap-y-1 max-[480px]:grid-cols-2">
            <label
              v-for="channel in channels"
              :key="channel.index"
              class="flex min-w-0 items-center gap-1.5"
            >
              <input v-model="chosen" type="checkbox" :value="channel.index" class="accent-ink" />
              <span class="truncate">{{ channel.label }}</span>
            </label>
          </div>
          <p v-if="!ready" role="alert" class="text-alert">{{ t('exportDialog.noChannel') }}</p>
        </fieldset>
      </div>
      <footer class="flex h-12 flex-none items-center justify-end gap-2 border-t border-line px-4">
        <button class="btn-ghost" @click="emit('close')">{{ t('exportDialog.cancel') }}</button>
        <button
          class="inline-flex h-7 items-center rounded-6 bg-ink px-3 font-medium text-canvas hover:opacity-86 disabled:opacity-55"
          :disabled="!ready"
          @click="run"
        >
          {{ t('exportDialog.export') }}
        </button>
      </footer>
    </div>
  </div>
</template>
