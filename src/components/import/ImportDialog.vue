<script setup lang="ts">
import { onKeyStroke, useElementSize } from '@vueuse/core'
import { computed, onMounted, shallowRef, useTemplateRef, watch } from 'vue'
import { useI18n } from 'vue-i18n'

import AppIcon from '@/components/common/AppIcon.vue'
import { getBackend } from '@/ipc/backend'
import { useCaptureStore } from '@/stores/capture'
import { formatBytes, formatCount, formatSamplerate, parseFrequency } from '@/utils/units'

import {
  allSignals,
  channelCount,
  chosenInOrder,
  defaultChoice,
  scopeState,
  signalsWithin,
  subtreeEnds,
  visibleRows,
} from './signalTree'

/**
 * The import dialog (DESIGN.md §11): the signals of a waveform file as a tree to choose from,
 * the sample grid they land on and the capture that makes. The file was read before the
 * dialog opened; importing lays the chosen signals out and shows the capture.
 */
const { t, locale } = useI18n()
const capture = useCaptureStore()
const dialog = useTemplateRef<HTMLElement>('dialog')
const list = useTemplateRef<HTMLElement>('list')
const { height: listHeight } = useElementSize(list)

const ROW = 24
const OVERSCAN = 8

const pending = computed(() => capture.pendingImport)
const items = computed(() => pending.value?.summary.items ?? [])
const ends = computed(() => subtreeEnds(items.value))
const chosen = shallowRef<Set<number>>(defaultChoice(items.value))
const collapsed = shallowRef<Set<number>>(new Set())
const query = shallowRef('')
const scrollTop = shallowRef(0)
/** The sample rate as typed: the one the timestamps give, unless the user knows better. */
const rateText = shallowRef(formatSamplerate(pending.value?.summary.samplerate ?? 0))

const fileName = computed(() => pending.value?.path.split(/[\\/]/).pop() ?? '')
const importing = computed(() => capture.job?.kind === 'import')
const progress = computed(() => (importing.value ? (capture.job?.progress ?? 0) : 0))

onKeyStroke('Escape', () => close())
onMounted(() => dialog.value?.focus())
watch(items, (next) => {
  chosen.value = defaultChoice(next)
  collapsed.value = new Set()
  rateText.value = formatSamplerate(pending.value?.summary.samplerate ?? 0)
})

// Virtual list: only the rows in sight, and a few more.
const rows = computed(() => visibleRows(items.value, ends.value, collapsed.value, query.value))
const visible = computed(() => {
  const first = Math.max(0, Math.floor(scrollTop.value / ROW) - OVERSCAN)
  const last = Math.min(
    rows.value.length,
    Math.ceil((scrollTop.value + listHeight.value) / ROW) + OVERSCAN,
  )
  return rows.value.slice(first, last).map((row, offset) => ({ row, top: (first + offset) * ROW }))
})

const channels = computed(() => channelCount(items.value, chosen.value))

/** The rate typed, when it is one. */
const rate = computed(() => parseFrequency(rateText.value))
/** Whether the rate typed is not the one the timestamps give. */
const custom = computed(
  () => rate.value !== null && rate.value !== pending.value?.summary.samplerate,
)
/**
 * The grid the rate makes, as the backend will lay it: whole time units per sample, from the
 * first timestamp to the last.
 */
const grid = computed(() => {
  const summary = pending.value?.summary
  if (!summary || rate.value === null) return null
  const step = Math.max(1, Math.round(1 / (rate.value * summary.unit)))
  return {
    samplerate: Math.round(1 / (step * summary.unit)),
    sampleCount: Math.floor(summary.span / step) + 1,
    coarsened: summary.finest % step !== 0,
  }
})
const ready = computed(() => channels.value > 0 && grid.value !== null && !importing.value)

// The capture is kept bit-packed in memory; past half of the machine's, warn.
const memory = shallowRef<number | null>(null)
onMounted(async () => {
  try {
    memory.value = await (await getBackend()).systemMemory()
  } catch (error) {
    console.error('memory size failed', error)
  }
})
const needed = computed(() => ((grid.value?.sampleCount ?? 0) * channels.value) / 8)
const tooLarge = computed(() => memory.value !== null && needed.value > memory.value / 2)

const outcome = computed(() => {
  if (!grid.value) return ''
  return t('importDialog.grid', {
    rate: formatSamplerate(grid.value.samplerate),
    samples: formatCount(grid.value.sampleCount, locale.value),
    channels: formatCount(channels.value, locale.value),
    size: formatBytes(needed.value),
  })
})

function resetRate(): void {
  rateText.value = formatSamplerate(pending.value?.summary.samplerate ?? 0)
}

function choose(signals: Iterable<number>, on: boolean): void {
  const next = new Set(chosen.value)
  for (const signal of signals) {
    if (on) next.add(signal)
    else next.delete(signal)
  }
  chosen.value = next
}

function chooseScope(index: number, on: boolean): void {
  choose(signalsWithin(items.value, ends.value, index), on)
}

function toggleFold(index: number): void {
  const next = new Set(collapsed.value)
  if (next.has(index)) next.delete(index)
  else next.add(index)
  collapsed.value = next
}

function checked(event: Event): boolean {
  return (event.target as HTMLInputElement).checked
}

/** Closes the dialog: stops the import under way, or forgets the file. */
function close(): void {
  if (importing.value) void capture.cancelFile()
  else void capture.discardImport()
}

async function run(): Promise<void> {
  if (!ready.value) return
  await capture.importWaveform({
    signals: chosenInOrder(items.value, chosen.value),
    samplerate: custom.value ? rate.value : null,
  })
}
</script>

<template>
  <div class="fixed inset-0 z-40 flex items-center justify-center modal-veil" @click.self="close">
    <div
      ref="dialog"
      role="dialog"
      aria-modal="true"
      :aria-label="t('importDialog.title')"
      tabindex="-1"
      class="flex h-[min(640px,85vh)] w-[min(560px,calc(100vw-32px))] flex-col rounded-6 bg-raise text-small outline-none"
    >
      <header
        class="flex h-10 flex-none items-center justify-between border-b border-line pr-2 pl-4"
      >
        <h2 class="flex min-w-0 items-baseline gap-2 font-medium">
          <span class="flex-none">{{ t('importDialog.title') }}</span>
          <span class="truncate font-normal text-ink-3">{{ fileName }}</span>
        </h2>
        <button class="btn-icon" :aria-label="t('importDialog.cancel')" @click="close">
          <AppIcon name="close" />
        </button>
      </header>
      <div v-if="pending" class="flex min-h-0 flex-1 flex-col gap-3 p-4">
        <div class="flex items-center gap-2">
          <span class="flex-1 text-caption text-ink-3">{{
            t(`importDialog.formats.${pending.summary.format}`)
          }}</span>
          <button
            class="text-caption text-ink-3 hover:text-ink"
            :disabled="importing"
            @click="choose(allSignals(items), true)"
          >
            {{ t('importDialog.all') }}
          </button>
          <button
            class="text-caption text-ink-3 hover:text-ink"
            :disabled="importing"
            @click="chosen = new Set()"
          >
            {{ t('importDialog.none') }}
          </button>
        </div>
        <label
          class="flex h-8 flex-none items-center gap-2 rounded-6 bg-control px-2 text-ink-2 focus-within:outline-2 focus-within:outline-offset-2 focus-within:outline-ink"
        >
          <AppIcon name="search" />
          <input
            v-model="query"
            class="min-w-0 flex-1 border-0 bg-transparent p-0 text-ink outline-none"
            :placeholder="t('importDialog.search')"
            :aria-label="t('importDialog.search')"
          />
        </label>
        <div
          ref="list"
          role="tree"
          :aria-label="t('importDialog.signals')"
          class="relative min-h-0 flex-1 overflow-y-auto rounded-6 bg-control"
          @scroll="scrollTop = list?.scrollTop ?? 0"
        >
          <p v-if="rows.length === 0" class="px-2 py-1.5 text-ink-3">
            {{ t('importDialog.noMatch') }}
          </p>
          <div v-else class="relative" :style="{ height: `${rows.length * ROW}px` }">
            <div
              v-for="{ row, top } in visible"
              :key="row.index"
              role="treeitem"
              :aria-level="row.item.depth + 1"
              :aria-expanded="row.item.kind === 'scope' ? !collapsed.has(row.index) : undefined"
              class="absolute inset-x-0 flex h-6 items-center gap-1.5 pr-2 hover:bg-hover"
              :style="{ top: `${top}px`, paddingLeft: `${8 + row.item.depth * 16}px` }"
            >
              <button
                v-if="row.item.kind === 'scope'"
                class="btn-icon h-5 w-5"
                :aria-label="
                  t(collapsed.has(row.index) ? 'importDialog.expand' : 'importDialog.collapse')
                "
                @click="toggleFold(row.index)"
              >
                <AppIcon :name="collapsed.has(row.index) ? 'chevron-right' : 'chevron-down'" />
              </button>
              <span v-else class="w-5 flex-none" aria-hidden="true" />
              <label
                v-if="row.item.kind === 'scope'"
                class="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5"
              >
                <input
                  type="checkbox"
                  class="accent-ink"
                  :checked="scopeState(items, ends, row.index, chosen) === 'all'"
                  :indeterminate="scopeState(items, ends, row.index, chosen) === 'some'"
                  :disabled="importing"
                  @change="chooseScope(row.index, checked($event))"
                />
                <span class="truncate font-medium">{{ row.item.name }}</span>
              </label>
              <label
                v-else-if="row.item.signal != null"
                class="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5"
              >
                <input
                  type="checkbox"
                  class="accent-ink"
                  :checked="chosen.has(row.item.signal)"
                  :disabled="importing"
                  @change="choose([row.item.signal], checked($event))"
                />
                <span class="truncate">{{ row.item.name }}</span>
                <span v-if="row.item.range" class="font-mono text-caption text-ink-3 tabular-nums"
                  >[{{ row.item.range[0] }}:{{ row.item.range[1] }}]</span
                >
                <span v-if="row.item.width > 1" class="text-caption text-ink-3 tabular-nums">{{
                  t('importDialog.bits', { count: row.item.width })
                }}</span>
              </label>
              <span v-else class="flex min-w-0 flex-1 items-center gap-1.5 text-ink-3">
                <span class="truncate">{{ row.item.name }}</span>
                <span class="text-caption">{{ t('importDialog.noLevel') }}</span>
              </span>
            </div>
          </div>
        </div>
        <div class="flex items-center gap-2">
          <label class="flex items-center gap-2">
            <span class="text-caption text-ink-3">{{ t('importDialog.samplerate') }}</span>
            <input
              v-model="rateText"
              class="h-7 w-28 rounded-4 bg-control px-2 text-ink tabular-nums outline-none focus:outline-2 focus:outline-offset-2 focus:outline-ink"
              :disabled="importing"
              @keydown.enter.prevent="run"
            />
          </label>
          <button v-if="custom" class="btn-ghost" :disabled="importing" @click="resetRate">
            {{ t('importDialog.autoRate') }}
          </button>
          <span v-if="rate === null" role="alert" class="text-caption text-alert">
            {{ t('importDialog.badSamplerate') }}
          </span>
        </div>
        <p class="text-caption text-ink-3 tabular-nums">{{ outcome }}</p>
        <p v-if="grid?.coarsened" class="text-caption text-ink-3">
          {{ t('importDialog.coarsened') }}
        </p>
        <p v-if="tooLarge" role="alert" class="rise-in text-caption text-alert tabular-nums">
          {{ t('importDialog.tooLarge', { size: formatBytes(needed) }) }}
        </p>
        <p v-if="channels === 0" role="alert" class="text-alert">
          {{ t('importDialog.noSignal') }}
        </p>
      </div>
      <footer class="flex h-12 flex-none items-center gap-2 border-t border-line px-4">
        <span
          v-if="importing"
          role="status"
          class="rise-in inline-flex flex-1 items-center gap-2 tabular-nums"
        >
          <span>{{ t('file.importing', { percent: Math.floor(progress * 100) }) }}</span>
          <span class="h-1 w-24 overflow-hidden rounded-full bg-hover" aria-hidden="true">
            <span class="block h-full bg-ink" :style="{ width: `${progress * 100}%` }" />
          </span>
        </span>
        <span v-else class="flex-1" />
        <button class="btn-ghost" @click="close">{{ t('importDialog.cancel') }}</button>
        <button
          class="inline-flex h-7 items-center rounded-6 bg-ink px-3 font-medium text-canvas hover:opacity-86 disabled:opacity-55"
          :disabled="!ready"
          @click="run"
        >
          {{ t('importDialog.import') }}
        </button>
      </footer>
    </div>
  </div>
</template>
